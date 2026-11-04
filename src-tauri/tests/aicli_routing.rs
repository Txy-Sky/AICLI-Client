#![cfg(feature = "test-hooks")]

mod support;

use axum::{extract::State, response::IntoResponse, Json, Router};
use cc_switch_lib::{
    aicli::routing_test_support::{
        self as routing_support, ProxyConfig, ProxyServer, RequestContext, RouteInput,
    },
    aicli::{
        alias,
        newapi::{NewApiClient, NewApiError},
        profiles::{ModelBinding, ProfileStore},
        resolver::{self, Resolved},
        runtime::{AicliRuntime, RuntimeRemote, RuntimeValues},
        secrets::{SecretError, SecretStore},
        seed,
        tokens::{groups_hash, TokenRemote},
    },
    AppType, Database,
};
use futures::future::BoxFuture;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

const UPSTREAM: &str = "upstream-fixture";
const PUBLIC: &str = "friendly-fixture";

#[derive(Default)]
struct Memory(Mutex<BTreeMap<String, String>>);
impl std::fmt::Debug for Memory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Memory { .. }")
    }
}
impl SecretStore for Memory {
    fn get(&self, key: &str) -> Result<Option<String>, SecretError> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn set(&self, key: &str, value: &str) -> Result<(), SecretError> {
        self.0.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<(), SecretError> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

struct Offline;
impl RuntimeRemote for Offline {
    fn fetch<'a>(
        &'a self,
        _: &'a NewApiClient,
    ) -> BoxFuture<'a, Result<RuntimeValues, NewApiError>> {
        Box::pin(async { Err(NewApiError::Network) })
    }
    fn tokens(&self, _: NewApiClient) -> Arc<dyn TokenRemote> {
        Arc::new(Self)
    }
}
impl TokenRemote for Offline {
    fn list(&self) -> BoxFuture<'_, Result<Vec<Value>, NewApiError>> {
        Box::pin(async { panic!("offline fixture must not list tokens") })
    }
    fn create<'a>(&'a self, _: &'a Value) -> BoxFuture<'a, Result<(), NewApiError>> {
        Box::pin(async { panic!("offline fixture must not create tokens") })
    }
    fn keys<'a>(
        &'a self,
        _: &'a [i64],
    ) -> BoxFuture<'a, Result<BTreeMap<i64, String>, NewApiError>> {
        Box::pin(async { panic!("offline fixture must not fetch keys") })
    }
}

#[derive(Debug, Clone)]
struct Captured {
    path: String,
    query: Option<String>,
    body: Value,
}
type Capture = Arc<Mutex<Vec<Captured>>>;

async fn upstream(
    State(capture): State<Capture>,
    req: axum::extract::Request,
) -> axum::response::Response {
    let path = req.uri().path().to_owned();
    let query = req.uri().query().map(str::to_owned);
    let bytes = axum::body::to_bytes(req.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    capture.lock().unwrap().push(Captured {
        path: path.clone(),
        query,
        body,
    });
    Json(if path.ends_with("messages") {
        json!({"id":"msg-fixture","type":"message","role":"assistant","model":UPSTREAM,"content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}})
    } else if path.ends_with("chat/completions") {
        json!({"id":"chatcmpl-fixture","object":"chat.completion","model":UPSTREAM,"choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}})
    } else if path.contains("/models/") {
        json!({"candidates":[{"content":{"role":"model","parts":[{"text":"ok"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":1,"totalTokenCount":2}})
    } else if path.contains("images/") || path.ends_with("alpha/search") {
        json!({"data":[],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}})
    } else {
        json!({"id":"resp-fixture","object":"response","model":UPSTREAM,"status":"completed","output":[{"id":"msg-fixture","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"ok","annotations":[]}]}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}})
    }).into_response()
}

struct Fixture {
    db: Arc<Database>,
    runtime: Arc<AicliRuntime>,
    proxy: ProxyServer,
    capture: Capture,
    mock: tokio::task::JoinHandle<()>,
    origin: String,
}
impl Fixture {
    async fn new(bound: bool) -> Self {
        support::ensure_test_home();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mock_origin = format!("http://{}", listener.local_addr().unwrap());
        let capture = Capture::default();
        let app = Router::new().fallback(upstream).with_state(capture.clone());
        let mock = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let db = Arc::new(Database::memory().unwrap());
        seed::init(&db).unwrap();
        let store = Arc::new(Memory::default());
        if bound {
            store
                .set("access-token", "synthetic-routing-access")
                .unwrap();
            store
                .set("token:17", "synthetic-routing-model-key")
                .unwrap();
            let conn = routing_support::connection(&db);
            conn.execute("INSERT INTO aicli_account(id,user_id,username,user_group) VALUES(1,17,'fixture','fixture')", []).unwrap();
            let groups = json!({"data":{"fixture":{"ratio":1}}});
            let pricing = json!({"auto_groups":["fixture"],"data":[{"model_name":UPSTREAM,"enable_groups":["fixture"],"supported_endpoint_types":["anthropic","openai","openai-response","gemini"]}]});
            for (kind, value) in [("groups", groups), ("pricing", pricing)] {
                conn.execute(
                    "INSERT INTO aicli_cache(kind,fetched_at,body_json) VALUES(?1,1,?2)",
                    rusqlite::params![kind, value.to_string()],
                )
                .unwrap();
            }
            conn.execute(r#"INSERT INTO aicli_managed_token VALUES(17,?1,'["fixture"]','fixture','masked',0)"#,[groups_hash(&["fixture".into()])]).unwrap();
        }
        let mut runtime = AicliRuntime::with_remote(db.clone(), store.clone(), Arc::new(Offline));
        if bound {
            let profiles = ProfileStore::new(db.clone());
            let snapshot = runtime.snapshot();
            let catalog = snapshot.catalog.as_ref().unwrap();
            let mut profile = profiles
                .copy_preset("recommended", "custom", "Custom", catalog)
                .unwrap();
            profile.body.model_bindings.push(ModelBinding {
                model: UPSTREAM.into(),
                source_group: Some("fixture".into()),
                alias: Some(PUBLIC.into()),
            });
            profiles.save(&profile, catalog).unwrap();
            routing_support::connection(&db)
                .execute("UPDATE aicli_account SET active_profile_id='custom'", [])
                .unwrap();
            runtime = AicliRuntime::with_remote(db.clone(), store, Arc::new(Offline));
        }
        // Substitute loopback transport DTOs; no resolver, binding, scope, alias,
        // credential lookup, or handler is mocked or bypassed. Desktop requires
        // synthetic fixture auth until production credential injection in C3-09.
        runtime.set_local_test_provider_source(move |app, line, wire| {
            let mut provider = seed::clone_for(app, line, wire).unwrap();
            match app {
                AppType::Claude | AppType::ClaudeDesktop => {
                    provider.settings_config["env"]["ANTHROPIC_BASE_URL"] = json!(mock_origin)
                }
                AppType::Gemini => {
                    provider.settings_config["env"]["GOOGLE_GEMINI_BASE_URL"] = json!(mock_origin)
                }
                _ => provider.settings_config["base_url"] = json!(mock_origin),
            }
            if matches!(app, AppType::ClaudeDesktop) {
                provider.settings_config["env"]["ANTHROPIC_AUTH_TOKEN"] =
                    json!("synthetic-desktop-fixture");
                // A deliberately conflicting legacy mapping must be ignored by AICLI.
                provider
                    .meta
                    .as_mut()
                    .unwrap()
                    .claude_desktop_model_routes
                    .insert(
                        UPSTREAM.into(),
                        serde_json::from_value(json!({"model":"must-not-map"})).unwrap(),
                    );
            }
            Some(provider)
        });
        let proxy = ProxyServer::new(
            ProxyConfig {
                listen_port: 0,
                enable_logging: false,
                ..Default::default()
            },
            db.clone(),
            None,
        )
        .with_test_runtime(runtime.clone());
        let info = proxy.start().await.unwrap();
        Self {
            db,
            runtime,
            proxy,
            capture,
            mock,
            origin: format!("http://127.0.0.1:{}", info.port),
        }
    }
    fn client(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap()
    }
    async fn stop(self) {
        self.proxy.stop().await.unwrap();
        self.mock.abort();
    }
}

#[tokio::test]
async fn unbound_all_inference_and_discovery_entries_return_401_without_forwarding() {
    let f = Fixture::new(false).await;
    for path in [
        "/v1/messages",
        "/claude-desktop/v1/messages",
        "/v1/chat/completions",
        "/v1/responses",
        "/v1/responses/compact",
        "/grokbuild/v1/responses",
        "/images/generations",
        "/images/edits",
        "/alpha/search",
        "/v1beta/models/friendly-fixture:generateContent",
    ] {
        let response = f
            .client()
            .post(format!("{}{path}", f.origin))
            .json(&json!({"model":PUBLIC,"messages":[],"input":[],"contents":[]}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 401, "{path}");
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"]["code"], "not_bound", "{path}: {body}");
    }
    for path in [
        "/v1/models",
        "/v1/models?limit=1000",
        "/claude-desktop/v1/models",
    ] {
        let response = f
            .client()
            .get(format!("{}{path}", f.origin))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 401, "{path}");
        assert_eq!(
            response.json::<Value>().await.unwrap()["error"]["code"],
            "not_bound"
        );
    }
    assert!(f.capture.lock().unwrap().is_empty());
    f.stop().await;
}

#[tokio::test]
async fn all_six_handler_families_rewrite_models_and_use_the_selected_profile() {
    let f = Fixture::new(true).await;
    let claude = alias::encode(&AppType::Claude, PUBLIC, Some(&f.db)).unwrap();
    let desktop = alias::encode(&AppType::ClaudeDesktop, PUBLIC, Some(&f.db)).unwrap();
    let gateway = routing_support::get_or_create_gateway_token(&f.db).unwrap();
    for (path, model, expected) in [
        ("/v1/messages", claude.as_str(), "/v1/messages"),
        (
            "/claude-desktop/v1/messages",
            desktop.as_str(),
            "/v1/messages",
        ),
        ("/v1/chat/completions", PUBLIC, "/v1/chat/completions"),
        ("/v1/responses", PUBLIC, "/v1/responses"),
        ("/v1/responses/compact", PUBLIC, "/v1/responses/compact"),
        ("/grokbuild/v1/responses", PUBLIC, "/v1/responses"),
        ("/images/generations", PUBLIC, "/v1/images/generations"),
        ("/images/edits", PUBLIC, "/v1/images/edits"),
        ("/alpha/search", PUBLIC, "/v1/alpha/search"),
    ] {
        f.capture.lock().unwrap().clear();
        let response=f.client().post(format!("{}{path}",f.origin)).header("x-api-key",&gateway).json(&json!({"model":model,"stream":false,"max_tokens":20,"messages":[{"role":"user","content":"hi"}],"input":"hi","prompt":"hi"})).send().await.unwrap();
        let status = response.status();
        let text = response.text().await.unwrap();
        assert!(status.is_success(), "{path}: {status} {text}");
        let captured = f.capture.lock().unwrap().clone();
        assert_eq!(captured.len(), 1, "{path}");
        assert_eq!(captured[0].path, expected, "{path}");
        assert_eq!(captured[0].body["model"], UPSTREAM, "{path}");
    }
    f.stop().await;
}

#[tokio::test]
async fn gemini_rewrites_action_and_get_model_urls_and_preserves_query() {
    let f = Fixture::new(true).await;
    for (prefix, action) in [
        ("/v1beta", ":generateContent"),
        ("/gemini/v1beta", ":countTokens"),
        ("/gemini/v1", ""),
    ] {
        f.capture.lock().unwrap().clear();
        let path = format!("{prefix}/models/{PUBLIC}{action}?alt=json&fixture=keep");
        let request = if action.is_empty() {
            f.client().get(format!("{}{path}", f.origin))
        } else {
            f.client()
                .post(format!("{}{path}", f.origin))
                .json(&json!({"contents":[{"role":"user","parts":[{"text":"hi"}]}]}))
        };
        let response = request.send().await.unwrap();
        let status = response.status();
        let text = response.text().await.unwrap();
        assert!(status.is_success(), "{path}: {status} {text}");
        let captured = f.capture.lock().unwrap().clone();
        assert_eq!(captured.len(), 1);
        assert!(
            captured[0]
                .path
                .ends_with(&format!("/models/{UPSTREAM}{action}")),
            "{:?}",
            captured[0]
        );
        assert_eq!(captured[0].query.as_deref(), Some("alt=json&fixture=keep"));
    }
    f.stop().await;
}

#[tokio::test]
async fn models_are_aliases_in_the_client_envelope_and_ignore_legacy_catalog_files() {
    let f = Fixture::new(true).await;
    let gateway = routing_support::get_or_create_gateway_token(&f.db).unwrap();
    let codex_dir = support::ensure_test_home().join(".codex");
    std::fs::create_dir_all(&codex_dir).unwrap();
    let sentinel = "legacy catalog must remain untouched\n";
    std::fs::write(codex_dir.join("config.toml"), sentinel).unwrap();
    for (path, app) in [
        ("/v1/models?limit=1000", AppType::Claude),
        ("/claude-desktop/v1/models", AppType::ClaudeDesktop),
        ("/v1/models", AppType::Codex),
    ] {
        let response = f
            .client()
            .get(format!("{}{path}", f.origin))
            .header("x-api-key", &gateway)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body: Value = response.json().await.unwrap();
        let data = body["data"].as_array().unwrap();
        assert_eq!(data.len(), 1, "{body}");
        assert_eq!(
            data[0]["id"],
            alias::encode(&app, PUBLIC, Some(&f.db)).unwrap()
        );
        if app == AppType::Codex {
            assert_eq!(body["object"], "list");
            assert_eq!(data[0]["object"], "model");
        } else {
            assert_eq!(data[0]["type"], "model");
            assert_eq!(body["has_more"], false);
        }
    }
    assert_eq!(
        std::fs::read_to_string(codex_dir.join("config.toml")).unwrap(),
        sentinel
    );
    assert!(f.capture.lock().unwrap().is_empty());
    f.stop().await;
}

#[tokio::test]
async fn misses_do_not_fall_back_to_the_legacy_provider() {
    let f = Fixture::new(true).await;
    let response = f
        .client()
        .post(format!("{}/v1/responses", f.origin))
        .json(&json!({"model":"missing","input":"hi"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    assert_eq!(
        response.json::<Value>().await.unwrap()["error"]["code"],
        "model_not_in_profile"
    );
    assert!(f.capture.lock().unwrap().is_empty());
    f.stop().await;
}

#[tokio::test]
async fn aicli_context_keeps_public_identity_retries_and_line_timeouts() {
    let f = Fixture::new(true).await;
    let mut body = json!({"model":PUBLIC});
    let headers = http::HeaderMap::new();
    let Resolved::Hit(decision) = resolver::resolve(
        &f.runtime,
        &AppType::Gemini,
        "gemini",
        &headers,
        &format!("/v1beta/models/{PUBLIC}:generateContent"),
        &mut body,
    )
    .unwrap() else {
        panic!("expected hit")
    };
    let state = f.proxy.test_state();
    let mut ctx = RequestContext::new(
        &state,
        &body,
        &headers,
        AppType::Gemini,
        "Gemini",
        "gemini",
        RouteInput::Aicli(decision),
    )
    .await
    .unwrap()
    .with_model_from_uri(
        &format!("/v1beta/models/{PUBLIC}:generateContent")
            .parse()
            .unwrap(),
    );
    assert!(!ctx.is_stack);
    assert!(ctx.app_config.auto_failover_enabled);
    assert_eq!(ctx.app_config.max_retries, 2);
    assert_eq!(ctx.current_provider_id, "aicli-main");
    assert_eq!(ctx.request_model, PUBLIC);
    assert_eq!(ctx.app_config.non_streaming_timeout, 110);
    assert_eq!(
        ctx.get_providers()
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        ["aicli-main", "aicli-global", "aicli-apac"]
    );
    assert_eq!(ctx.streaming_timeout_config().first_byte_timeout, 100);
    ctx.provider = ctx.get_providers()[2].clone();
    assert_eq!(ctx.streaming_timeout_config().first_byte_timeout, 300);
    assert_eq!(ctx.streaming_timeout_config().idle_timeout, 300);
    f.stop().await;
}

#[tokio::test]
async fn discovery_follows_scope_changes_but_existing_sessions_keep_their_pin() {
    let f = Fixture::new(true).await;
    let session = "fixture-session-0000000000001";
    let response = f
        .client()
        .post(format!("{}/v1/messages", f.origin))
        .header("x-claude-code-session-id", session)
        .json(&json!({"model":PUBLIC,"max_tokens":20,"messages":[{"role":"user","content":"hi"}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    routing_support::connection(&f.db)
        .execute(
            "INSERT INTO aicli_scope_binding VALUES('app','claude','recommended')",
            [],
        )
        .unwrap();
    for (pinned, expected) in [(true, PUBLIC), (false, UPSTREAM)] {
        let mut request = f.client().get(format!("{}/v1/models?limit=1000", f.origin));
        if pinned {
            request = request.header("x-claude-code-session-id", session);
        }
        let body: Value = request.send().await.unwrap().json().await.unwrap();
        assert_eq!(
            body["data"][0]["id"],
            alias::encode(&AppType::Claude, expected, Some(&f.db)).unwrap()
        );
    }
    f.stop().await;
}

#[tokio::test]
async fn transport_test_hook_rejects_non_loopback_without_connecting() {
    let f = Fixture::new(true).await;
    f.runtime.set_local_test_provider_source(|app, line, _| {
        Some(seed::clone_for(app, line, "anthropic").unwrap())
    });
    let mut body = json!({"model":PUBLIC});
    assert!(resolver::resolve(
        &f.runtime,
        &AppType::Claude,
        "anthropic",
        &http::HeaderMap::new(),
        "/v1/messages",
        &mut body
    )
    .is_err());
    assert!(f.capture.lock().unwrap().is_empty());
    f.stop().await;
}
