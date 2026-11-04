//! Resolve one immutable profile snapshot, then prepare fixed-line Provider DTOs.
use std::{collections::HashMap, fmt, sync::Arc, time::Instant};

use http::{HeaderMap, StatusCode};
use rusqlite::OptionalExtension;
use serde_json::Value;

use super::{
    alias,
    catalog::Compatibility,
    lines::{Line, LINES},
    runtime::{AicliRuntime, RoutesSnapshot, RuntimeError},
    seed,
};
use crate::{app_config::AppType, provider::Provider, proxy::session::extract_session_id};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineTimeouts {
    pub first_byte_s: u64,
    pub idle_s: u64,
    pub stream_total_s: u64,
    pub non_stream_s: u64,
}

impl From<&Line> for LineTimeouts {
    fn from(line: &Line) -> Self {
        Self {
            first_byte_s: line.first_byte_s,
            idle_s: line.idle_s,
            stream_total_s: line.stream_total_s,
            non_stream_s: line.non_stream_s,
        }
    }
}

pub struct RouteDecision {
    pub public_model: String,
    pub upstream_model: String,
    pub profile_id: String,
    pub token_id: i64,
    pub credential: String,
    /// Empty for server-generated sessions: never send an affinity header.
    pub session_key: String,
    pub wire_protocol: String,
    pub providers: Vec<Provider>,
    pub timeouts: HashMap<String, LineTimeouts>,
}

impl fmt::Debug for RouteDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RouteDecision")
            .field("public_model", &self.public_model)
            .field("upstream_model", &self.upstream_model)
            .field("profile_id", &self.profile_id)
            .field("token_id", &self.token_id)
            .field("wire_protocol", &self.wire_protocol)
            .field("timeouts", &self.timeouts)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub enum Resolved {
    NotBound,
    Plain,
    Hit(Box<RouteDecision>),
    Miss(RouteMiss),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteMiss {
    ModelNotInProfile,
    NeedsRepair,
    Unsupported,
    NotBound,
}

impl RouteMiss {
    pub fn code(self) -> &'static str {
        match self {
            Self::ModelNotInProfile => "model_not_in_profile",
            Self::NeedsRepair => "profile_needs_repair",
            Self::Unsupported => "capability_unsupported",
            Self::NotBound => "not_bound",
        }
    }

    pub fn response(self, app: &AppType) -> (StatusCode, Value) {
        let message = match self {
            Self::ModelNotInProfile => "Model is not in the selected profile",
            Self::NeedsRepair => "Profile needs repair before this model can be used",
            Self::Unsupported => "Model does not support this client capability",
            Self::NotBound => "Bind an account before sending model requests",
        };
        let mut body = crate::proxy::stack_miss_body(app, message);
        body["error"]["code"] = Value::String(self.code().into());
        (
            if self == Self::NotBound {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::BAD_REQUEST
            },
            body,
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error(transparent)]
    Alias(#[from] alias::AliasError),
    #[error("AICLI routing storage is unavailable")]
    Storage,
}

fn profile_id(
    runtime: &AicliRuntime,
    snapshot: &RoutesSnapshot,
    app: &AppType,
    session: Option<&str>,
) -> Result<String, ResolveError> {
    let conn = runtime
        .database()
        .conn
        .lock()
        .map_err(|_| ResolveError::Storage)?;
    // v1 deliberately skips project bindings: proxy requests have no project identity.
    for (scope, key) in session
        .map(|id| ("session", id))
        .into_iter()
        .chain([("app", app.as_str()), ("global", "")])
    {
        let id: Option<String> = conn
            .query_row(
                "SELECT profile_id FROM aicli_scope_binding WHERE scope=?1 AND scope_key=?2",
                [scope, key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| ResolveError::Storage)?;
        if let Some(id) = id {
            return Ok(id);
        }
    }
    Ok(snapshot
        .active_profile_id
        .clone()
        .unwrap_or_else(|| "recommended".into()))
}

fn ordered_lines(
    runtime: &AicliRuntime,
    preferred: &str,
    model: &str,
    now: Instant,
) -> Vec<&'static Line> {
    let line_cooldowns = runtime
        .cooldown
        .lines
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let model_cooldowns = runtime
        .cooldown
        .models
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut lines: Vec<_> = LINES.iter().collect();
    lines.sort_by_key(|line| (line.id != preferred, std::cmp::Reverse(line.priority)));
    let recovery = |line: &&Line| {
        line_cooldowns
            .get(line.id)
            .copied()
            .into_iter()
            .chain(
                model_cooldowns
                    .get(&(line.id.to_owned(), model.to_owned()))
                    .copied(),
            )
            .filter(|until| *until > now)
            .max()
    };
    let ready: Vec<_> = lines
        .iter()
        .copied()
        .filter(|line| recovery(line).is_none())
        .collect();
    if !ready.is_empty() {
        return ready;
    }
    // Both cooldown levels must have expired before a line is usable. Pick
    // exactly one probe with the earliest effective recovery (stable ties).
    lines.into_iter().min_by_key(recovery).into_iter().collect()
}

/// No HTTP, client-file writes or fallback to a legacy Provider. Protocol is
/// supplied by the request endpoint, independently of the client app identity.
pub fn resolve(
    runtime: &AicliRuntime,
    app: &AppType,
    client_protocol: &str,
    headers: &HeaderMap,
    uri: &str,
    body: &mut Value,
) -> Result<Resolved, ResolveError> {
    if !runtime.bound_for_routing()? {
        return Ok(Resolved::NotBound);
    }
    let model = if client_protocol == "gemini" {
        uri.split('?')
            .next()
            .and_then(|path| path.rsplit_once("/models/"))
            .and_then(|(_, tail)| tail.rsplit_once(':'))
            .map(|(model, _)| model)
    } else {
        body.get("model").and_then(Value::as_str)
    };
    let Some(model) = model.filter(|model| !model.is_empty()) else {
        return Ok(Resolved::Plain);
    };
    let public_model =
        alias::decode(app, model, Some(runtime.database()))?.unwrap_or_else(|| model.to_owned());
    let session = extract_session_id(headers, body, app.as_str());
    // Include app identity and use unambiguous framing. The wire affinity key
    // remains the owner-specified <profile_id>:<session_id>.
    let pin_key = serde_json::to_string(&(app.as_str(), &session.session_id))
        .map_err(|_| ResolveError::Storage)?;
    let pin = session
        .client_provided
        .then(|| runtime.session_pin(&pin_key))
        .flatten();
    let new_pin_key = (session.client_provided && pin.is_none()).then_some(pin_key);
    let (snapshot, selected) = match pin {
        Some(pin) => (pin.snapshot, pin.profile_id),
        None => {
            let snapshot = runtime.snapshot();
            let selected = profile_id(
                runtime,
                &snapshot,
                app,
                session
                    .client_provided
                    .then_some(session.session_id.as_str()),
            )?;
            (snapshot, selected)
        }
    };
    resolve_profile(
        runtime,
        app,
        client_protocol,
        body,
        &public_model,
        snapshot,
        selected,
        &session,
        new_pin_key,
    )
}

#[allow(clippy::too_many_arguments)]
fn resolve_profile(
    runtime: &AicliRuntime,
    app: &AppType,
    protocol: &str,
    body: &mut Value,
    public_model: &str,
    snapshot: Arc<RoutesSnapshot>,
    selected: String,
    session: &crate::proxy::SessionIdResult,
    pin_key: Option<String>,
) -> Result<Resolved, ResolveError> {
    let Some(entry) = snapshot.profiles.get(&selected) else {
        return Ok(Resolved::Miss(RouteMiss::ModelNotInProfile));
    };
    // Repair takes precedence over lookup: revoked exclusive models may have
    // disappeared from the current catalog and compiled route list entirely.
    if entry.compiled.repair.needs_repair
        && entry
            .compiled
            .repair
            .affected_models
            .iter()
            .any(|model| model == public_model)
    {
        return Ok(Resolved::Miss(RouteMiss::NeedsRepair));
    }
    let Some(route) = entry
        .compiled
        .routes
        .iter()
        .find(|route| route.public_model == public_model)
    else {
        return Ok(Resolved::Miss(RouteMiss::ModelNotInProfile));
    };
    let Some((token_id, credential)) = entry.token_ids.get(public_model).and_then(|id| {
        runtime
            .credential(*id)
            .filter(|key| !key.is_empty())
            .map(|key| (*id, key))
    }) else {
        return Ok(Resolved::Miss(RouteMiss::NotBound));
    };
    let Some(model) = snapshot
        .catalog
        .as_ref()
        .and_then(|catalog| catalog.models.get(&route.upstream_model))
    else {
        return Ok(Resolved::Miss(RouteMiss::Unsupported));
    };
    if model.compat.get(protocol) == Some(&Compatibility::Unavailable) {
        return Ok(Resolved::Miss(RouteMiss::Unsupported));
    }
    let Some(wire) = model.wire.get(protocol) else {
        return Ok(Resolved::Miss(RouteMiss::Unsupported));
    };
    let preferred = if entry.profile.body.line_policy == "inherit" {
        &snapshot.selected_line_id
    } else {
        &entry.profile.body.line_policy
    };
    let mut providers = Vec::new();
    let mut timeouts = HashMap::new();
    for line in ordered_lines(runtime, preferred, &route.upstream_model, Instant::now()) {
        let Ok(provider) = seed::clone_for(app, line.id, wire) else {
            return Ok(Resolved::Miss(RouteMiss::Unsupported));
        };
        timeouts.insert(provider.id.clone(), LineTimeouts::from(line));
        providers.push(provider);
    }
    let decision = RouteDecision {
        public_model: public_model.into(),
        upstream_model: route.upstream_model.clone(),
        profile_id: selected.clone(),
        token_id,
        credential,
        session_key: if session.client_provided {
            format!("{}:{}", selected, session.session_id)
        } else {
            String::new()
        },
        wire_protocol: wire.clone(),
        providers,
        timeouts,
    };
    body["model"] = Value::String(decision.upstream_model.clone());
    if let Some(pin_key) = pin_key {
        runtime.pin(pin_key, selected, snapshot);
    }
    Ok(Resolved::Hit(Box::new(decision)))
}

#[cfg(test)]
mod tests;
