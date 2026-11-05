//! HTTP-only account API. No browser cookies, user-id header, redirects or raw error bodies.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::time::Duration;

use reqwest::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Client, Method, Url};
use serde_json::{json, Value};

use super::lines::{line, LINES};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const PAGE_SIZE: u64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NewApiError {
    #[error("System access token is invalid; bind the account again")]
    TokenInvalid,
    #[error("Account API returned HTTP {0}")]
    Http(u16),
    #[error("Account API rate limited the operation; retry after {0} seconds")]
    RateLimited(u64),
    #[error("Account API rejected the operation")]
    Rejected,
    #[error("Account API returned an invalid response")]
    InvalidResponse,
    #[error("Account API is unreachable")]
    Network,
    #[error("Invalid account API input")]
    InvalidInput,
    #[error("Unknown service line")]
    UnknownLine,
}

#[derive(Clone)]
pub struct NewApiClient {
    client: Client,
    origins: Vec<Url>,
    authorization: HeaderValue,
}

impl fmt::Debug for NewApiClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NewApiClient { .. }")
    }
}

impl NewApiClient {
    pub fn new(access_token: &str, selected_line: &str) -> Result<Self, NewApiError> {
        let selected = line(selected_line).ok_or(NewApiError::UnknownLine)?;
        let origins = std::iter::once(selected)
            .chain(LINES.iter().filter(|entry| entry.id != selected_line))
            .map(|entry| Url::parse(entry.base_url).map_err(|_| NewApiError::InvalidInput))
            .collect::<Result<_, _>>()?;
        Self::build(access_token, origins, false)
    }

    fn build(access_token: &str, origins: Vec<Url>, local: bool) -> Result<Self, NewApiError> {
        if access_token.is_empty() || origins.is_empty() {
            return Err(NewApiError::InvalidInput);
        }
        let mut authorization = HeaderValue::from_str(&format!("Bearer {access_token}"))
            .map_err(|_| NewApiError::InvalidInput)?;
        authorization.set_sensitive(true);
        let builder = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none());
        let client = if local { builder.no_proxy() } else { builder }
            .build()
            .map_err(|_| NewApiError::Network)?;
        Ok(Self {
            client,
            origins,
            authorization,
        })
    }

    /// Only compiled for isolated CI fixtures; production callers select a fixed line ID.
    #[cfg(feature = "test-hooks")]
    pub fn for_local_test(access_token: &str, origin: &str) -> Result<Self, NewApiError> {
        let url = Url::parse(origin).map_err(|_| NewApiError::InvalidInput)?;
        if url.scheme() != "http"
            || !matches!(url.host_str(), Some("127.0.0.1" | "localhost"))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(NewApiError::InvalidInput);
        }
        Self::build(access_token, vec![url], true)
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> Result<Value, NewApiError> {
        let mut last_error = NewApiError::Network;
        for origin in &self.origins {
            let url = origin.join(path).map_err(|_| NewApiError::InvalidInput)?;
            let mut request = self
                .client
                .request(method.clone(), url)
                .header(AUTHORIZATION, self.authorization.clone())
                .query(query);
            if let Some(body) = body {
                request = request.json(body);
            }
            let response = match request.send().await {
                Ok(response) => response,
                Err(_) => {
                    last_error = NewApiError::Network;
                    continue;
                }
            };
            let status = response.status();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err(NewApiError::TokenInvalid);
            }
            // Reject every 4xx before interpreting a body; never change lines for auth/quota errors.
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                if let Some(delay) = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|value| value.to_str().ok())
                    .and_then(retry_after_seconds)
                {
                    return Err(NewApiError::RateLimited(delay));
                }
            }
            if status.is_client_error() {
                return Err(NewApiError::Http(status.as_u16()));
            }
            let html_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.to_ascii_lowercase().contains("text/html"));
            let bytes = match response.bytes().await {
                Ok(bytes) => bytes,
                Err(_) => {
                    last_error = NewApiError::Network;
                    continue;
                }
            };
            let prefix = String::from_utf8_lossy(&bytes[..bytes.len().min(256)])
                .trim_start()
                .to_ascii_lowercase();
            if status.is_server_error()
                && (html_type
                    || prefix.starts_with("<!doctype html")
                    || prefix.starts_with("<html"))
            {
                last_error = NewApiError::Http(status.as_u16());
                continue;
            }
            if !status.is_success() {
                return Err(NewApiError::Http(status.as_u16()));
            }
            let result: Value =
                serde_json::from_slice(&bytes).map_err(|_| NewApiError::InvalidResponse)?;
            return match result.get("success").and_then(Value::as_bool) {
                Some(true) => Ok(result),
                Some(false) => Err(NewApiError::Rejected),
                None => Err(NewApiError::InvalidResponse),
            };
        }
        Err(last_error)
    }

    pub async fn user_self(&self) -> Result<Value, NewApiError> {
        self.request(Method::GET, "/api/user/self", &[], None).await
    }
    pub async fn user_groups(&self) -> Result<Value, NewApiError> {
        self.request(Method::GET, "/api/user/self/groups", &[], None)
            .await
    }
    pub async fn pricing(&self) -> Result<Value, NewApiError> {
        // Keep top-level group_ratio/usable_group/supported_endpoint information as well as data.
        self.request(Method::GET, "/api/pricing", &[], None).await
    }
    pub async fn auto_groups_setting(&self) -> Result<Value, NewApiError> {
        self.request(Method::GET, "/api/token/auto-groups", &[], None)
            .await
    }
    pub async fn list_tokens(&self) -> Result<Vec<Value>, NewApiError> {
        let mut items = Vec::new();
        let mut seen = HashSet::new();
        let mut page = 1_u64;
        loop {
            let result = self
                .request(
                    Method::GET,
                    "/api/token/",
                    &[
                        ("p", page.to_string()),
                        ("page_size", PAGE_SIZE.to_string()),
                    ],
                    None,
                )
                .await?;
            let data = result.get("data").ok_or(NewApiError::InvalidResponse)?;
            let total = data
                .get("total")
                .and_then(Value::as_u64)
                .ok_or(NewApiError::InvalidResponse)?;
            let batch = data
                .get("items")
                .and_then(Value::as_array)
                .ok_or(NewApiError::InvalidResponse)?;
            if batch.is_empty() && (items.len() as u64) < total {
                return Err(NewApiError::InvalidResponse);
            }
            for item in batch {
                let id = item
                    .get("id")
                    .and_then(Value::as_i64)
                    .filter(|id| *id > 0)
                    .ok_or(NewApiError::InvalidResponse)?;
                if !seen.insert(id) {
                    return Err(NewApiError::InvalidResponse);
                }
                items.push(item.clone());
            }
            if items.len() as u64 >= total {
                return Ok(items);
            }
            page = page.checked_add(1).ok_or(NewApiError::InvalidResponse)?;
        }
    }
    pub async fn create_token(&self, specification: &Value) -> Result<(), NewApiError> {
        self.request(Method::POST, "/api/token/", &[], Some(specification))
            .await
            .map(|_| ())
    }
    pub async fn delete_token(&self, token_id: i64) -> Result<(), NewApiError> {
        if token_id <= 0 {
            return Err(NewApiError::InvalidInput);
        }
        self.request(Method::DELETE, &format!("/api/token/{token_id}"), &[], None)
            .await
            .map(|_| ())
    }
    pub async fn batch_token_keys(
        &self,
        ids: &[i64],
    ) -> Result<BTreeMap<i64, String>, NewApiError> {
        if ids.iter().any(|id| *id <= 0) {
            return Err(NewApiError::InvalidInput);
        }
        let result = self
            .request(
                Method::POST,
                "/api/token/batch/keys",
                &[],
                Some(&json!({"ids": ids})),
            )
            .await?;
        let keys = result
            .pointer("/data/keys")
            .and_then(Value::as_object)
            .ok_or(NewApiError::InvalidResponse)?;
        ids.iter()
            .map(|id| {
                let key = keys
                    .get(&id.to_string())
                    .and_then(Value::as_str)
                    .filter(|key| !key.is_empty())
                    .ok_or(NewApiError::InvalidResponse)?;
                Ok((*id, key.to_owned()))
            })
            .collect()
    }
    pub async fn log_by_request_id(&self, request_id: &str) -> Result<Option<Value>, NewApiError> {
        if request_id.is_empty() {
            return Err(NewApiError::InvalidInput);
        }
        let result = self
            .request(
                Method::GET,
                "/api/log/self",
                &[
                    ("request_id", request_id.to_owned()),
                    ("p", "1".into()),
                    ("page_size", "1".into()),
                ],
                None,
            )
            .await?;
        let items = result
            .pointer("/data/items")
            .and_then(Value::as_array)
            .ok_or(NewApiError::InvalidResponse)?;
        match items.as_slice() {
            [] => Ok(None),
            [item] if item.get("request_id").and_then(Value::as_str) == Some(request_id) => {
                Ok(Some(item.clone()))
            }
            _ => Err(NewApiError::InvalidResponse),
        }
    }
    fn time_query(start: i64, end: i64) -> Result<Vec<(&'static str, String)>, NewApiError> {
        if start < 0 || end < start {
            return Err(NewApiError::InvalidInput);
        }
        Ok(vec![
            ("type", "2".into()),
            ("start_timestamp", start.to_string()),
            ("end_timestamp", end.to_string()),
        ])
    }
    pub async fn log_stat(&self, start: i64, end: i64) -> Result<Value, NewApiError> {
        self.request(
            Method::GET,
            "/api/log/self/stat",
            &Self::time_query(start, end)?,
            None,
        )
        .await
    }
    pub async fn log_count(&self, start: i64, end: i64) -> Result<u64, NewApiError> {
        let mut query = Self::time_query(start, end)?;
        query.extend([("p", "1".into()), ("page_size", "1".into())]);
        let result = self
            .request(Method::GET, "/api/log/self", &query, None)
            .await?;
        result
            .pointer("/data/total")
            .and_then(Value::as_u64)
            .ok_or(NewApiError::InvalidResponse)
    }

    /// Read one day's consume rows so quota and request count come from the same bounded API.
    pub async fn log_daily_usage(&self, start: i64, end: i64) -> Result<Value, NewApiError> {
        let mut page = 1_u64;
        let mut requests = 0_i64;
        let mut quota = 0_i64;
        let mut seen = HashSet::new();
        loop {
            let mut query = Self::time_query(start, end)?;
            query.extend([
                ("p", page.to_string()),
                ("page_size", PAGE_SIZE.to_string()),
            ]);
            let result = self
                .request(Method::GET, "/api/log/self", &query, None)
                .await?;
            let data = result.get("data").ok_or(NewApiError::InvalidResponse)?;
            let total = data
                .get("total")
                .and_then(Value::as_u64)
                .ok_or(NewApiError::InvalidResponse)?;
            let items = data
                .get("items")
                .and_then(Value::as_array)
                .ok_or(NewApiError::InvalidResponse)?;
            if items.is_empty() && (requests as u64) < total {
                return Err(NewApiError::InvalidResponse);
            }
            for item in items {
                let id = item
                    .get("id")
                    .and_then(Value::as_i64)
                    .filter(|id| *id > 0)
                    .ok_or(NewApiError::InvalidResponse)?;
                if !seen.insert(id) {
                    return Err(NewApiError::InvalidResponse);
                }
                quota = quota
                    .checked_add(
                        item.get("quota")
                            .and_then(Value::as_i64)
                            .ok_or(NewApiError::InvalidResponse)?,
                    )
                    .ok_or(NewApiError::InvalidResponse)?;
                requests = requests
                    .checked_add(1)
                    .ok_or(NewApiError::InvalidResponse)?;
            }
            if requests as u64 > total {
                return Err(NewApiError::InvalidResponse);
            }
            if (requests as u64) >= total {
                return Ok(json!({"data":{"quota":quota,"requests":requests}}));
            }
            page = page.checked_add(1).ok_or(NewApiError::InvalidResponse)?;
        }
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub async fn audit_self_request_count(&self, start: i64, end: i64) -> Result<u64, NewApiError> {
        if start < 0 || end < start {
            return Err(NewApiError::InvalidInput);
        }
        let result = self
            .request(
                Method::GET,
                "/api/audit/self",
                &[
                    ("category", "access_token".into()),
                    ("start_timestamp", start.to_string()),
                    ("end_timestamp", end.to_string()),
                    ("p", "1".into()),
                    ("page_size", "100".into()),
                ],
                None,
            )
            .await?;
        result
            .pointer("/data/total")
            .and_then(Value::as_u64)
            .ok_or(NewApiError::InvalidResponse)
    }
}

fn retry_after_seconds(value: &str) -> Option<u64> {
    value.trim().parse().ok().or_else(|| {
        chrono::DateTime::parse_from_rfc2822(value)
            .ok()
            .map(|date| {
                date.timestamp()
                    .saturating_sub(chrono::Utc::now().timestamp())
                    .max(0) as u64
            })
    })
}

/// Unauthenticated `/api/status` reachability probe for `aicli_probe_lines`.
/// Deliberately not a `NewApiClient` method: it must work while unbound, target
/// exactly one line (no failover to neighbours), and never attach auth headers.
pub async fn public_status(line_id: &str) -> Result<(u16, Value), NewApiError> {
    let entry = line(line_id).ok_or(NewApiError::UnknownLine)?;
    let client = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| NewApiError::Network)?;
    let url = Url::parse(entry.base_url)
        .map_err(|_| NewApiError::InvalidInput)?
        .join("/api/status")
        .map_err(|_| NewApiError::InvalidInput)?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|_| NewApiError::Network)?;
    let status = response.status();
    if !status.is_success() {
        return Err(NewApiError::Http(status.as_u16()));
    }
    let bytes = response.bytes().await.map_err(|_| NewApiError::Network)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| NewApiError::InvalidResponse)?;
    match value.get("success").and_then(Value::as_bool) {
        Some(true) => Ok((status.as_u16(), value)),
        Some(false) => Err(NewApiError::Rejected),
        None => Err(NewApiError::InvalidResponse),
    }
}

#[cfg(test)]
mod tests;
