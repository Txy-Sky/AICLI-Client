use super::*;
use crate::{
    proxy::{providers::get_adapter, server::ProxyServer, types::ProxyConfig},
    services::ProviderService,
    store::AppState,
};
use axum::{extract::State, response::IntoResponse, Json, Router};
use serde_json::Value;
use serial_test::serial;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    sync::{Arc, Mutex},
};

struct Home {
    dir: tempfile::TempDir,
    previous: Vec<(&'static str, Option<OsString>)>,
}

impl Home {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let previous = ["HOME", "USERPROFILE", "CC_SWITCH_TEST_HOME"]
            .into_iter()
            .map(|name| {
                let old = std::env::var_os(name);
                std::env::set_var(name, dir.path());
                (name, old)
            })
            .collect();
        crate::settings::reload_settings().unwrap();
        Self { dir, previous }
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        for (name, value) in &self.previous {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
        crate::settings::reload_settings().unwrap();
    }
}

#[test]
#[serial]
fn seeds_are_keyless_idempotent_hidden_and_do_not_touch_client_files() {
    let home = Home::new();
    let originals: BTreeMap<_, _> = [
        ".claude/settings.json",
        ".codex/config.toml",
        ".gemini/.env",
        ".cc-switch/config.json",
    ]
    .into_iter()
    .map(|path| (path, format!("unchanged fixture: {path}\n").into_bytes()))
    .collect();
    for (path, bytes) in &originals {
        let path = home.dir.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    let db = Arc::new(Database::memory().unwrap());
    let existing = Provider::with_id("existing".into(), "Existing".into(), json!({}), None);
    db.save_provider("claude", &existing).unwrap();
    db.set_current_provider("claude", "existing").unwrap();
    assert_eq!(init(&db).unwrap(), 15);
    assert_eq!(init(&db).unwrap(), 0);
    let state = AppState::new(db.clone());
    for app in AppType::all() {
        let all = db.get_all_providers(app.as_str()).unwrap();
        let seeds: Vec<_> = all.values().filter(|p| is_hidden(p)).collect();
        assert_eq!(seeds.len(), if app.has_route_mode() { 3 } else { 0 });
        if !app.has_route_mode() {
            continue;
        }
        for line in &LINES {
            let provider = all.get(&format!("aicli-{}", line.id)).unwrap();
            assert_eq!(
                provider.settings_config,
                base_provider(&app, line.id).unwrap().settings_config
            );
            assert_eq!(
                get_adapter(&app)
                    .unwrap()
                    .extract_base_url(provider)
                    .unwrap(),
                line.base_url
            );
            assert!(get_adapter(&app).unwrap().extract_auth(provider).is_none());
            assert!(!provider.in_failover_queue);
            assert_eq!(provider.meta.as_ref().unwrap().api_format, None);
        }
        assert!(ProviderService::list(&state, app.clone())
            .unwrap()
            .values()
            .all(|p| !is_hidden(p)));
        assert_eq!(
            crate::mode::current::provider_for(&db, &app, crate::mode::current::Purpose::Direct)
                .unwrap(),
            if app == AppType::Claude {
                Some("existing".into())
            } else {
                None
            }
        );
    }
    // Missing internal rows are repaired; unrelated rows and selected pointers survive.
    db.delete_provider("gemini", "aicli-apac").unwrap();
    assert_eq!(init(&db).unwrap(), 1);
    for (path, bytes) in originals {
        assert_eq!(std::fs::read(home.dir.path().join(path)).unwrap(), bytes);
    }
    assert!(!home.dir.path().join(".claude-desktop").exists());
}

#[test]
fn clones_validate_inputs_and_never_mutate_the_seed() {
    let db = Database::memory().unwrap();
    init(&db).unwrap();
    for app in AppType::all().filter(AppType::has_route_mode) {
        let wires: &[&str] = match app {
            AppType::Claude | AppType::ClaudeDesktop => {
                &["anthropic", "openai", "openai-response", "gemini"]
            }
            AppType::Codex | AppType::GrokBuild => &["anthropic", "openai", "openai-response"],
            _ => &["gemini"],
        };
        for line in &LINES {
            let original = db
                .get_provider_by_id(&format!("aicli-{}", line.id), app.as_str())
                .unwrap()
                .unwrap();
            for wire in wires {
                let clone = clone_for(&app, line.id, wire).unwrap();
                assert_eq!(clone.id, original.id);
                assert_eq!(clone.settings_config, original.settings_config);
                assert!(get_adapter(&app).unwrap().extract_auth(&clone).is_none());
                assert!(clone.meta.unwrap().api_format.is_some());
            }
            let unchanged = db
                .get_provider_by_id(&original.id, app.as_str())
                .unwrap()
                .unwrap();
            assert_eq!(
                serde_json::to_value(unchanged).unwrap(),
                serde_json::to_value(original).unwrap()
            );
        }
    }
    assert!(clone_for(&AppType::Pi, "main", "openai").is_err());
    assert!(clone_for(&AppType::Claude, "untrusted", "anthropic").is_err());
    assert!(clone_for(&AppType::Claude, "main", "untrusted").is_err());
    assert!(clone_for(&AppType::Gemini, "main", "openai").is_err());
}

type Capture = Arc<Mutex<Vec<(String, Value)>>>;

async fn mock_upstream(
    State(capture): State<Capture>,
    req: axum::extract::Request,
) -> axum::response::Response {
    let path = req.uri().path().to_string();
    let bytes = axum::body::to_bytes(req.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    capture.lock().unwrap().push((path.clone(), body));
    let response = if path.ends_with("/messages") {
        json!({"id":"msg-seed","type":"message","role":"assistant","model":"seed-model","content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}})
    } else if path.ends_with("/chat/completions") {
        json!({"id":"chatcmpl-seed","object":"chat.completion","model":"seed-model","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}})
    } else if path.ends_with("/responses") {
        json!({"id":"resp-seed","object":"response","model":"seed-model","status":"completed","output":[{"id":"msg-seed","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"ok","annotations":[]}]}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}})
    } else {
        json!({"candidates":[{"content":{"role":"model","parts":[{"text":"ok"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":1,"totalTokenCount":2}})
    };
    Json(response).into_response()
}

async fn forwards(app: AppType, wire: &str, expected_path: &str) {
    let _home = Home::new();
    let capture = Capture::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let router = Router::new()
        .fallback(mock_upstream)
        .with_state(capture.clone());
    let mock = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let db = Arc::new(Database::memory().unwrap());
    init(&db).unwrap();
    let mut provider = clone_for(&app, "main", wire).unwrap();
    // Loopback and a synthetic credential exist only in this isolated fixture.
    match app {
        AppType::Claude | AppType::ClaudeDesktop => {
            provider.settings_config["env"]["ANTHROPIC_BASE_URL"] = json!(origin);
            provider.settings_config["env"]["ANTHROPIC_AUTH_TOKEN"] =
                json!("synthetic-seed-fixture");
        }
        AppType::Codex | AppType::GrokBuild => {
            provider.settings_config["base_url"] = json!(origin);
            provider.settings_config["auth"] = json!({"OPENAI_API_KEY":"synthetic-seed-fixture"});
        }
        AppType::Gemini => {
            provider.settings_config["env"]["GOOGLE_GEMINI_BASE_URL"] = json!(origin);
            provider.settings_config["env"]["GEMINI_API_KEY"] = json!("synthetic-seed-fixture");
        }
        _ => unreachable!(),
    }
    db.save_provider(app.as_str(), &provider).unwrap();
    db.set_current_provider(app.as_str(), &provider.id).unwrap();
    let local_token = crate::aicli::local_auth::token_for(&db, app.as_str()).unwrap();
    let proxy = ProxyServer::bound_forwarding_fixture(
        ProxyConfig {
            listen_port: 0,
            enable_logging: false,
            non_streaming_timeout: 10,
            ..Default::default()
        },
        db,
        None,
    );
    let info = proxy.start().await.unwrap();
    let (endpoint, body) = match app {
        AppType::Claude => (
            "/v1/messages",
            json!({"model":"seed-model","max_tokens":20,"stream":false,"messages":[{"role":"user","content":"hello"}]}),
        ),
        AppType::Codex => (
            "/v1/responses",
            json!({"model":"seed-model","stream":false,"input":[{"role":"user","content":[{"type":"input_text","text":"hello"}]}]}),
        ),
        AppType::Gemini => (
            "/v1beta/models/seed-model:generateContent",
            json!({"contents":[{"role":"user","parts":[{"text":"hello"}]}]}),
        ),
        _ => unreachable!(),
    };
    let response = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap()
        .post(format!("http://127.0.0.1:{}{endpoint}", info.port))
        .bearer_auth(local_token)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let response_body = response.text().await.unwrap();
    assert!(
        status.is_success(),
        "{app:?}/{wire}: {status} {response_body}"
    );
    let requests = capture.lock().unwrap().clone();
    assert_eq!(requests.len(), 1, "{app:?}/{wire}: {requests:?}");
    assert_eq!(requests[0].0, expected_path, "{app:?}/{wire}");
    let sent = &requests[0].1;
    match wire {
        "anthropic" | "openai" => assert!(sent["messages"].is_array()),
        "openai-response" => assert!(sent.get("input").is_some()),
        "gemini" => assert!(sent["contents"].is_array()),
        _ => unreachable!(),
    }
    proxy.stop().await.unwrap();
    mock.abort();
}

macro_rules! forwarding_test {
    ($name:ident, $app:ident, $wire:literal, $path:literal) => {
        #[tokio::test]
        #[serial]
        async fn $name() {
            forwards(AppType::$app, $wire, $path).await;
        }
    };
}

forwarding_test!(claude_anthropic, Claude, "anthropic", "/v1/messages");
forwarding_test!(claude_chat, Claude, "openai", "/v1/chat/completions");
forwarding_test!(claude_responses, Claude, "openai-response", "/v1/responses");
forwarding_test!(
    claude_gemini,
    Claude,
    "gemini",
    "/v1beta/models/seed-model:generateContent"
);
forwarding_test!(codex_responses, Codex, "openai-response", "/v1/responses");
forwarding_test!(codex_chat, Codex, "openai", "/v1/chat/completions");
forwarding_test!(codex_anthropic, Codex, "anthropic", "/v1/messages");
forwarding_test!(
    gemini_native,
    Gemini,
    "gemini",
    "/v1beta/models/seed-model:generateContent"
);
