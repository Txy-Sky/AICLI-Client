//! Local-only credentials. They are never valid upstream credentials.
use crate::{database::Database, error::AppError};
use axum::{extract::State, http::StatusCode, middleware::Next, response::Response};
use rusqlite::OptionalExtension;
use subtle::ConstantTimeEq;

pub fn token_for(db: &Database, app: &str) -> Result<String, AppError> {
    let conn = db
        .conn
        .lock()
        .map_err(|e| AppError::Database(e.to_string()))?;
    if let Some(token) = conn
        .query_row(
            "SELECT token FROM aicli_local_token WHERE app_type=?1",
            [app],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok(token);
    }
    let token = format!("aicli-local-{}", uuid::Uuid::new_v4().simple());
    conn.execute(
        "INSERT INTO aicli_local_token(app_type,token,created_at) VALUES(?1,?2,?3)",
        rusqlite::params![app, token, chrono::Utc::now().timestamp()],
    )?;
    Ok(token)
}

pub fn is_proxy_token(value: &str) -> bool {
    value.starts_with("aicli-local-") || value == "PROXY_MANAGED"
}

pub fn matches(db: &Database, candidates: &[String]) -> Result<bool, AppError> {
    let conn = db
        .conn
        .lock()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let mut statement = conn.prepare("SELECT token FROM aicli_local_token")?;
    let tokens = statement.query_map([], |row| row.get::<_, String>(0))?;
    let mut matched = 0u8;
    for token in tokens {
        let token = token?;
        for candidate in candidates {
            matched |= candidate.as_bytes().ct_eq(token.as_bytes()).unwrap_u8();
        }
    }
    Ok(matched == 1)
}

/// Like [`matches`], but also tells which client the matching local token was
/// issued to (F-09 privacy records). Every token is still compared in constant time.
pub fn matched_app(db: &Database, candidates: &[String]) -> Result<Option<String>, AppError> {
    let conn = db
        .conn
        .lock()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let mut statement = conn.prepare("SELECT app_type, token FROM aicli_local_token")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut found = None;
    for row in rows {
        let (app, token) = row?;
        let mut matched = 0u8;
        for candidate in candidates {
            matched |= candidate.as_bytes().ct_eq(token.as_bytes()).unwrap_u8();
        }
        if matched == 1 && found.is_none() {
            found = Some(app);
        }
    }
    Ok(found)
}

pub async fn authenticate(
    State(state): State<crate::proxy::server::ProxyState>,
    mut request: axum::extract::Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if request.headers().get_all("origin").iter().any(|origin| {
        !matches!(
            origin.to_str(),
            Ok("tauri://") | Ok("http://tauri.localhost")
        )
    }) {
        return Err(StatusCode::FORBIDDEN);
    }
    if matches!(request.uri().path(), "/health" | "/status") {
        return Ok(next.run(request).await);
    }
    let mut candidates = Vec::new();
    for header in request.headers().get_all("authorization") {
        if let Ok(value) = header.to_str() {
            if let Some((scheme, token)) = value.split_once(' ') {
                if scheme.eq_ignore_ascii_case("bearer") {
                    candidates.push(token.trim().to_string());
                }
            }
        }
    }
    for name in ["x-api-key", "x-goog-api-key"] {
        candidates.extend(
            request
                .headers()
                .get_all(name)
                .iter()
                .filter_map(|value| value.to_str().ok().map(str::to_string)),
        );
    }
    let gemini =
        request.uri().path().contains("/v1beta/") || request.uri().path().starts_with("/gemini/");
    if gemini {
        candidates.extend(
            url::form_urlencoded::parse(request.uri().query().unwrap_or_default().as_bytes())
                .filter(|(key, _)| key == "key")
                .map(|(_, value)| value.into_owned()),
        );
    }
    let Some(app) =
        matched_app(&state.db, &candidates).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    else {
        return Err(StatusCode::UNAUTHORIZED);
    };
    // F-09: the privacy layer records which client a masked request came from.
    request
        .extensions_mut()
        .insert(crate::aicli::privacy::events::ClientApp(app));
    for name in ["authorization", "x-api-key", "x-goog-api-key"] {
        request.headers_mut().remove(name);
    }
    // Strip the SDK query credential too; never forward a local token in the URL.
    if gemini {
        let pairs: Vec<_> =
            url::form_urlencoded::parse(request.uri().query().unwrap_or_default().as_bytes())
                .filter(|(key, _)| key != "key")
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect();
        let query = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(pairs)
            .finish();
        let uri = if query.is_empty() {
            request.uri().path().to_string()
        } else {
            format!("{}?{query}", request.uri().path())
        };
        *request.uri_mut() = uri.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
    }
    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request, routing::any, Router};
    use std::sync::Arc;

    #[test]
    fn tokens_persist_and_are_distinct_per_app() {
        let db = Database::memory().unwrap();
        let a = token_for(&db, "claude").unwrap();
        assert_eq!(a, token_for(&db, "claude").unwrap());
        assert_ne!(a, token_for(&db, "codex").unwrap());
        assert_eq!(a.len(), 44);
        assert!(a[12..]
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)));
        assert!(is_proxy_token(&a));
        assert!(is_proxy_token("PROXY_MANAGED"));
        assert!(!is_proxy_token("real-upstream"));
    }

    #[tokio::test]
    async fn middleware_authenticates_all_apps_strips_credentials_and_blocks_origins() {
        let db = Arc::new(Database::memory().unwrap());
        let state = crate::proxy::server::ProxyState::for_test(db.clone());
        let router = Router::new()
            .fallback(any(|request: axum::extract::Request| async move {
                for name in ["authorization", "x-api-key", "x-goog-api-key"] {
                    assert!(!request.headers().contains_key(name));
                }
                assert!(!request.uri().to_string().contains("aicli-local-"));
                if request.uri().path().starts_with("/v1beta/") {
                    assert_eq!(request.uri().query(), Some("alt=sse"));
                }
                StatusCode::OK
            }))
            .layer(axum::middleware::from_fn_with_state(state, authenticate));
        for app in ["claude", "codex", "gemini", "grokbuild", "claude-desktop"] {
            let token = token_for(&db, app).unwrap();
            for header in ["authorization", "x-api-key", "x-goog-api-key", "query"] {
                let path = if header == "query" {
                    format!("/v1beta/models/model:generateContent?key={token}&alt=sse")
                } else {
                    "/v1/messages".into()
                };
                let mut request = Request::builder().uri(path);
                if header != "query" {
                    request = request.header(
                        header,
                        if header == "authorization" {
                            format!("Bearer {token}")
                        } else {
                            token.clone()
                        },
                    );
                }
                let response =
                    tower::Service::call(&mut router.clone(), request.body(Body::empty()).unwrap())
                        .await
                        .unwrap();
                assert_eq!(response.status(), StatusCode::OK);
            }
        }
        for (path, auth, origin, expected) in [
            ("/v1/messages", None, None, 401),
            ("/v1/messages", Some("wrong"), None, 401),
            ("/v1/messages", Some("PROXY_MANAGED"), None, 401),
            ("/health", None, None, 200),
            ("/status", None, None, 200),
            ("/v1/messages", None, Some("https://evil.example"), 403),
            (
                "/health",
                None,
                Some("http://tauri.localhost.evil.example"),
                403,
            ),
            ("/health", None, Some("tauri://"), 200),
            ("/health", None, Some("http://tauri.localhost"), 200),
        ] {
            let mut request = Request::builder().uri(path);
            if let Some(auth) = auth {
                request = request.header("x-api-key", auth);
            }
            if let Some(origin) = origin {
                request = request.header("origin", origin);
            }
            let response =
                tower::Service::call(&mut router.clone(), request.body(Body::empty()).unwrap())
                    .await
                    .unwrap();
            assert_eq!(response.status().as_u16(), expected);
        }
    }
}
