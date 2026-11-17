//! One credential per ordered group list. Only HTTP manages remote tokens;
//! SQLite stores public metadata and SecretStore owns all credential values.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use futures::future::BoxFuture;
use rusqlite::params;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::newapi::{NewApiClient, NewApiError};
use super::secrets::{SecretError, SecretStore};
use crate::Database;

pub const CREATED_TOKEN_LIMIT: usize = 32;
const REUSE_LIFETIME: i64 = 30 * 24 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CleanupCandidate {
    pub name: String,
    pub masked: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TokenError {
    #[error("Invalid ordered groups")]
    InvalidGroups,
    #[error("Auto groups are unavailable; a multi-group token is required")]
    AutoGroupUnavailable,
    #[error("Client-created token limit reached; clean up unused tokens on the website")]
    TokenLimit { unused: Vec<CleanupCandidate> },
    #[error("Token operation is deferred until {retry_at}")]
    RateLimited { retry_at: i64 },
    #[error("Local token metadata is unavailable")]
    Database,
    #[error("Token recovery has already been attempted")]
    RecoveryExhausted,
    #[error(transparent)]
    Api(#[from] NewApiError),
    #[error(transparent)]
    Secret(#[from] SecretError),
}

/// Injection boundary for counting real HTTP operations in isolated acceptance.
/// Implementations must not log or expose request/response credential values.
pub trait TokenRemote: Send + Sync {
    fn list(&self) -> BoxFuture<'_, Result<Vec<Value>, NewApiError>>;
    fn create<'a>(&'a self, specification: &'a Value) -> BoxFuture<'a, Result<(), NewApiError>>;
    fn keys<'a>(
        &'a self,
        ids: &'a [i64],
    ) -> BoxFuture<'a, Result<BTreeMap<i64, String>, NewApiError>>;
}

impl TokenRemote for NewApiClient {
    fn list(&self) -> BoxFuture<'_, Result<Vec<Value>, NewApiError>> {
        Box::pin(self.list_tokens())
    }
    fn create<'a>(&'a self, specification: &'a Value) -> BoxFuture<'a, Result<(), NewApiError>> {
        Box::pin(self.create_token(specification))
    }
    fn keys<'a>(
        &'a self,
        ids: &'a [i64],
    ) -> BoxFuture<'a, Result<BTreeMap<i64, String>, NewApiError>> {
        Box::pin(self.batch_token_keys(ids))
    }
}

#[derive(Clone)]
struct Record {
    id: i64,
    hash: String,
    groups: Vec<String>,
    name: String,
    masked: String,
    created: bool,
}

#[derive(Default)]
struct Operation {
    retry_at: Option<i64>,
    recovered: BTreeSet<i64>,
}

pub struct TokenService {
    db: Arc<Database>,
    secrets: Arc<dyn SecretStore>,
    remote: Arc<dyn TokenRemote>,
    operation: tokio::sync::Mutex<Operation>,
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
}

impl fmt::Debug for TokenService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TokenService { .. }")
    }
}

pub fn groups_hash(groups: &[String]) -> String {
    format!("{:x}", Sha256::digest(groups.join("\n").as_bytes()))
}

fn valid_groups(groups: &[String]) -> bool {
    !groups.is_empty()
        && groups
            .iter()
            .all(|g| !g.is_empty() && !g.contains(['\r', '\n']))
        && groups.iter().collect::<BTreeSet<_>>().len() == groups.len()
}

fn group_list(value: &Value) -> Option<Vec<String>> {
    let parsed;
    let value = if let Some(text) = value.as_str() {
        parsed = serde_json::from_str::<Value>(text).ok()?;
        &parsed
    } else {
        value
    };
    value
        .as_array()?
        .iter()
        .map(|g| g.as_str().map(str::to_owned))
        .collect()
}

fn reusable(candidate: &Value, groups: &[String], now: i64) -> bool {
    let no_ips = match candidate.get("allow_ips") {
        None | Some(Value::Null) => true,
        Some(Value::String(value)) => value.trim().is_empty(),
        Some(Value::Array(value)) => value.is_empty(),
        _ => false,
    };
    candidate["id"].as_i64().is_some_and(|id| id > 0)
        && candidate["status"] == 1
        && candidate["expired_time"]
            .as_i64()
            .is_some_and(|time| time == -1 || time > now.saturating_add(REUSE_LIFETIME))
        && candidate["unlimited_quota"] == true
        && candidate["model_limits_enabled"] == false
        && no_ips
        && if groups.len() == 1 {
            candidate["group"].as_str() == Some(groups[0].as_str())
        } else {
            candidate["group"] == "auto"
                && candidate["cross_group_retry"] == true
                && group_list(&candidate["auto_groups"]).as_deref() == Some(groups)
        }
}

fn preferred<'a>(tokens: &'a [Value], groups: &[String], now: i64) -> Option<&'a Value> {
    tokens
        .iter()
        .filter(|token| reusable(token, groups, now))
        .min_by_key(|token| {
            (
                !token["name"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with("aicli:"),
                token["id"].as_i64().unwrap_or(i64::MAX),
            )
        })
}

impl TokenService {
    pub fn new(
        db: Arc<Database>,
        secrets: Arc<dyn SecretStore>,
        remote: Arc<dyn TokenRemote>,
    ) -> Self {
        Self {
            db,
            secrets,
            remote,
            operation: Default::default(),
            clock: Arc::new(|| chrono::Utc::now().timestamp()),
        }
    }

    fn records(&self) -> Result<Vec<Record>, TokenError> {
        let conn = self.db.conn.lock().map_err(|_| TokenError::Database)?;
        let mut statement = conn.prepare("SELECT token_id,groups_hash,groups_json,name,masked,created_by_client FROM aicli_managed_token ORDER BY token_id").map_err(|_| TokenError::Database)?;
        let rows = statement
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get::<_, String>(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })
            .map_err(|_| TokenError::Database)?;
        rows.map(|row| {
            let (id, hash, groups, name, masked, created) =
                row.map_err(|_| TokenError::Database)?;
            Ok(Record {
                id,
                hash,
                groups: serde_json::from_str(&groups).map_err(|_| TokenError::Database)?,
                name,
                masked,
                created,
            })
        })
        .collect()
    }

    fn save(&self, record: &Record) -> Result<(), TokenError> {
        let groups =
            serde_json::to_string(&record.groups).map_err(|_| TokenError::InvalidGroups)?;
        self.db.conn.lock().map_err(|_| TokenError::Database)?.execute(
            "INSERT INTO aicli_managed_token(token_id,groups_hash,groups_json,name,masked,created_by_client) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(groups_hash) DO UPDATE SET token_id=excluded.token_id,groups_json=excluded.groups_json,name=excluded.name,masked=excluded.masked,created_by_client=excluded.created_by_client",
            params![record.id,record.hash,groups,record.name,record.masked,record.created]).map_err(|_| TokenError::Database)?;
        Ok(())
    }

    fn api_error(&self, operation: &mut Operation, error: NewApiError) -> TokenError {
        let delay = match error {
            NewApiError::RateLimited(delay) => delay,
            NewApiError::Http(429) => 60,
            _ => return error.into(),
        };
        let retry_at =
            (self.clock)().saturating_add(i64::try_from(delay.max(1)).unwrap_or(i64::MAX));
        operation.retry_at = Some(retry_at);
        log::warn!("AICLI Token operation rate limited; deferred until {retry_at}");
        TokenError::RateLimited { retry_at }
    }

    /// Run once per compiled snapshot, batching all distinct group orders. The
    /// caller supplies reference orders from *all* local profiles, including
    /// repair baselines, so cleanup suggestions never label another profile's
    /// token unused. No remote token is ever updated or deleted here.
    pub async fn ensure_compilation(
        &self,
        ordered_groups: &[Vec<String>],
        auto_available: bool,
        all_profile_references: &[Vec<String>],
    ) -> Result<BTreeMap<String, i64>, TokenError> {
        let mut operation = self.operation.lock().await;
        self.ensure_locked(
            ordered_groups,
            auto_available,
            all_profile_references,
            &mut operation,
        )
        .await
    }

    pub async fn ensure_token(
        &self,
        groups: &[String],
        auto_available: bool,
        all_profile_references: &[Vec<String>],
    ) -> Result<i64, TokenError> {
        self.ensure_compilation(&[groups.to_vec()], auto_available, all_profile_references)
            .await?
            .get(&groups_hash(groups))
            .copied()
            .ok_or(TokenError::Database)
    }

    async fn ensure_locked(
        &self,
        requested: &[Vec<String>],
        auto_available: bool,
        references: &[Vec<String>],
        operation: &mut Operation,
    ) -> Result<BTreeMap<String, i64>, TokenError> {
        if requested
            .iter()
            .chain(references)
            .any(|groups| !valid_groups(groups))
        {
            return Err(TokenError::InvalidGroups);
        }
        if !auto_available && requested.iter().any(|groups| groups.len() > 1) {
            return Err(TokenError::AutoGroupUnavailable);
        }
        let unique: BTreeMap<_, _> = requested
            .iter()
            .map(|groups| (groups_hash(groups), groups.clone()))
            .collect();
        let records = self.records()?;
        let mut result = BTreeMap::new();
        let mut missing = Vec::new();
        for (hash, groups) in &unique {
            if let Some(record) = records
                .iter()
                .find(|r| r.hash == *hash && r.groups == *groups)
            {
                if self
                    .secrets
                    .get(&format!("token:{}", record.id))?
                    .is_some_and(|key| !key.is_empty())
                {
                    result.insert(hash.clone(), record.id);
                    continue;
                }
            }
            missing.push((hash.clone(), groups.clone()));
        }
        if missing.is_empty() {
            return Ok(result);
        }
        let now = (self.clock)();
        if let Some(retry_at) = operation.retry_at.filter(|time| *time > now) {
            return Err(TokenError::RateLimited { retry_at });
        }
        operation.retry_at = None;
        let mut tokens = self
            .remote
            .list()
            .await
            .map_err(|e| self.api_error(operation, e))?;
        let new_count = missing
            .iter()
            .filter(|(_, groups)| preferred(&tokens, groups, now).is_none())
            .count();
        if records.iter().filter(|r| r.created).count() + new_count > CREATED_TOKEN_LIMIT {
            let referenced: BTreeSet<_> = references
                .iter()
                .chain(requested)
                .map(|groups| groups_hash(groups))
                .collect();
            let unused = tokens
                .iter()
                .filter_map(|candidate| {
                    let name = candidate["name"].as_str()?;
                    if !name.starts_with("aicli:") {
                        return None;
                    }
                    let groups = if candidate["group"] == "auto" {
                        group_list(&candidate["auto_groups"])?
                    } else {
                        vec![candidate["group"].as_str()?.to_owned()]
                    };
                    if referenced.contains(&groups_hash(&groups)) {
                        return None;
                    }
                    let masked = records
                        .iter()
                        .find(|r| Some(r.id) == candidate["id"].as_i64())
                        .map(|r| r.masked.clone())
                        .unwrap_or_else(|| "sk-…".into());
                    Some(CleanupCandidate {
                        name: name.to_owned(),
                        masked,
                    })
                })
                .collect();
            return Err(TokenError::TokenLimit { unused });
        }
        let mut plans = Vec::new();
        for (hash, groups) in missing {
            let mut created = false;
            let token = if let Some(token) = preferred(&tokens, &groups, now) {
                token.clone()
            } else {
                let base = format!("aicli:{}", &hash[..8]);
                let mut name = base.clone();
                let mut suffix = 2_u64;
                while tokens
                    .iter()
                    .any(|t| t["name"].as_str() == Some(name.as_str()))
                {
                    name = format!("{base}-{suffix}");
                    suffix = suffix.checked_add(1).ok_or(TokenError::InvalidGroups)?;
                }
                let specification = if groups.len() == 1 {
                    json!({"name":name,"group":groups[0],"unlimited_quota":true,"expired_time":-1})
                } else {
                    json!({"name":name,"group":"auto","auto_groups":groups,"cross_group_retry":true,"unlimited_quota":true,"expired_time":-1})
                };
                self.remote
                    .create(&specification)
                    .await
                    .map_err(|e| self.api_error(operation, e))?;
                tokens = self
                    .remote
                    .list()
                    .await
                    .map_err(|e| self.api_error(operation, e))?;
                created = true;
                tokens
                    .iter()
                    .filter(|t| t["name"] == name && reusable(t, &groups, now))
                    .min_by_key(|t| t["id"].as_i64().unwrap_or(i64::MAX))
                    .cloned()
                    .ok_or(NewApiError::InvalidResponse)?
            };
            let id = token["id"].as_i64().ok_or(NewApiError::InvalidResponse)?;
            let record = Record {
                id,
                hash,
                groups,
                name: token["name"]
                    .as_str()
                    .ok_or(NewApiError::InvalidResponse)?
                    .to_owned(),
                masked: "sk-…".into(),
                created: created || records.iter().any(|r| r.id == id && r.created),
            };
            // Keep ownership/limit metadata even if the subsequent key fetch is
            // rate limited or SecretStore fails. Retrying can reuse this token.
            self.save(&record)?;
            plans.push(record);
        }
        let ids: Vec<_> = plans
            .iter()
            .map(|r| r.id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let keys = self
            .remote
            .keys(&ids)
            .await
            .map_err(|e| self.api_error(operation, e))?;
        for mut record in plans {
            let raw = keys
                .get(&record.id)
                .filter(|s| !s.is_empty())
                .ok_or(NewApiError::InvalidResponse)?;
            let key = if raw.starts_with("sk-") {
                raw.clone()
            } else {
                format!("sk-{raw}")
            };
            if key.len() <= 7 {
                return Err(NewApiError::InvalidResponse.into());
            }
            self.secrets.set(&format!("token:{}", record.id), &key)?;
            let tail: String = key
                .chars()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            record.masked = format!("sk-…{tail}");
            self.save(&record)?;
            result.insert(record.hash, record.id);
        }
        Ok(result)
    }

    /// The request layer calls this once after a model-key 401. Rebuild only
    /// local metadata/credentials; never revoke the system access token or
    /// delete a user-owned remote token. It performs no request replay itself.
    pub async fn recover_unauthorized(
        &self,
        id: i64,
        groups: &[String],
        auto_available: bool,
        references: &[Vec<String>],
    ) -> Result<i64, TokenError> {
        if !valid_groups(groups) {
            return Err(TokenError::InvalidGroups);
        }
        let mut operation = self.operation.lock().await;
        let hash = groups_hash(groups);
        if let Some(current) = self
            .records()?
            .iter()
            .find(|r| r.hash == hash && r.id != id)
        {
            if self
                .secrets
                .get(&format!("token:{}", current.id))?
                .is_some()
            {
                return Ok(current.id);
            }
        }
        if !operation.recovered.insert(id) {
            return Err(TokenError::RecoveryExhausted);
        }
        let account = format!("token:{id}");
        let old = self.secrets.get(&account)?;
        {
            let mut conn = self.db.conn.lock().map_err(|_| TokenError::Database)?;
            let transaction = conn.transaction().map_err(|_| TokenError::Database)?;
            transaction
                .execute("DELETE FROM aicli_managed_token WHERE token_id=?1", [id])
                .map_err(|_| TokenError::Database)?;
            self.secrets.delete(&account)?;
            if transaction.commit().is_err() {
                if let Some(key) = old {
                    self.secrets.set(&account, &key)?;
                }
                return Err(TokenError::Database);
            }
        }
        self.ensure_locked(
            &[groups.to_vec()],
            auto_available,
            references,
            &mut operation,
        )
        .await?
        .get(&hash)
        .copied()
        .ok_or(TokenError::Database)
    }
}

#[cfg(test)]
mod tests;

#[cfg(feature = "test-hooks")]
pub mod acceptance;
