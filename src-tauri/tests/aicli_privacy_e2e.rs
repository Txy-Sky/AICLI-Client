//! E-06 端到端验收：真实代理 + mock 上游（原始字节录制）。
//!
//! 覆盖清单：四协议遮盖/还原（流式与非流式）、关闭隐私逐字节相同、
//! Claude→Responses 协议转换、重启后占位符不变、故障注入 3×2、
//! 层顺序（鉴权先于隐私）、流式进行中并发遮盖、媒体接口、压缩流
//! fail-closed、日志不含敏感值。

#![cfg(feature = "test-hooks")]

mod support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use axum::response::IntoResponse;
use cc_switch_lib::aicli::privacy::layer::{FAULT_MASK, FAULT_RESTORE_JSON, FAULT_STREAM};
use cc_switch_lib::aicli::routing_test_support::{
    self as routing_support, ProxyConfig, ProxyServer,
};
use cc_switch_lib::aicli::{
    alias,
    newapi::{NewApiClient, NewApiError},
    profiles::{ModelBinding, ProfileStore},
    runtime::{AicliRuntime, RuntimeRemote, RuntimeValues},
    secrets::{SecretError, SecretStore},
    seed,
    tokens::{groups_hash, TokenRemote},
};
use cc_switch_lib::{AppType, Database};
use futures::future::BoxFuture;
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::sync::Notify;

const UPSTREAM: &str = "upstream-fixture";
const PUBLIC: &str = "friendly-fixture";

// ---- 敏感值（运行时拼接构造，避免源码字面量） ----

fn email_value() -> String {
    [
        "li", "ming", "@", "mail", ".", "acme", "-", "corp", ".", "cn",
    ]
    .concat()
}

fn sk_key() -> String {
    ["sk-", "Zx9QvLtYb3Nc8UmKe4Hd"].concat()
}

// ---- 日志捕获（日志不含敏感值断言） ----

fn captured_logs() -> &'static Mutex<Vec<String>> {
    static LOGS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    LOGS.get_or_init(|| {
        struct Capture;
        impl log::Log for Capture {
            fn enabled(&self, _: &log::Metadata) -> bool {
                true
            }
            fn log(&self, record: &log::Record) {
                if let Ok(mut buffer) = captured_logs().try_lock() {
                    buffer.push(format!("{}", record.args()));
                }
            }
            fn flush(&self) {}
        }
        let _ = log::set_boxed_logger(Box::new(Capture));
        log::set_max_level(log::LevelFilter::Info);
        Mutex::new(Vec::new())
    })
}

// ---- mock 上游：原始字节录制 + 可中途切换的响应模式 ----

#[derive(Clone)]
enum UpstreamMode {
    /// 各协议 JSON（text 字段用 replacement）。
    Json,
    /// 各协议 JSON，text 回显请求体里出现的第一个 {{…}} 占位符
    ///（验证响应还原：客户端应拿到原值）。
    JsonEcho,
    /// SSE 文本按给定字节切点流式下发。
    Sse(String, Vec<usize>),
    /// 先发首块，等 release 再发尾块（并发测试用）。
    SseHeld(String, usize, Arc<Notify>),
    /// gzip 压缩 SSE（fail-closed 测试用）。
    SseCompressed(String),
}

type Mode = Arc<Mutex<UpstreamMode>>;
type Replacement = Arc<Mutex<String>>;

#[derive(Clone)]
struct Captured {
    path: String,
    headers: http::HeaderMap,
    body: Vec<u8>,
}
type Capture = Arc<Mutex<Vec<Captured>>>;

async fn upstream(
    axum::extract::State(state): axum::extract::State<(Capture, Mode, Replacement)>,
    req: axum::extract::Request,
) -> axum::response::Response {
    let (capture, mode, replacement) = state;
    let path = req.uri().path().to_owned();
    let headers = req.headers().clone();
    let bytes = axum::body::to_bytes(req.into_body(), 64 * 1024 * 1024)
        .await
        .unwrap()
        .to_vec();
    capture.lock().unwrap().push(Captured {
        path: path.clone(),
        headers,
        body: bytes.clone(),
    });
    let mode = mode.lock().unwrap().clone();
    match mode {
        UpstreamMode::Json => json_response(&path, &replacement.lock().unwrap().clone()),
        UpstreamMode::JsonEcho => {
            let text = std::str::from_utf8(bytes.as_slice()).unwrap();
            let echo = text
                .find("{{")
                .map(|start| {
                    let end = text[start..].find("}}").unwrap() + start + 2;
                    text[start..end].to_owned()
                })
                .unwrap_or_else(|| "no-placeholder".to_owned());
            json_response(&path, &echo)
        }
        UpstreamMode::Sse(text, cuts) => {
            let mut response = axum::response::Response::builder()
                .header("content-type", "text/event-stream")
                .body(axum::body::Body::from_stream(sse_chunks(text, cuts)))
                .unwrap();
            response
                .headers_mut()
                .insert("x-oneapi-request-id", "up-fixture".parse().unwrap());
            response
        }
        UpstreamMode::SseHeld(text, first_cut, release) => {
            let first_cut = first_cut.min(text.len());
            let head = text[..first_cut].to_owned();
            let tail = text[first_cut..].to_owned();
            let release = release.clone();
            let stream = async_stream::stream! {
                yield Ok::<_, std::io::Error>(bytes::Bytes::from(head.to_owned()));
                release.notified().await;
                yield Ok(bytes::Bytes::from(tail.to_owned()));
            };
            axum::response::Response::builder()
                .header("content-type", "text/event-stream")
                .body(axum::body::Body::from_stream(stream))
                .unwrap()
        }
        UpstreamMode::SseCompressed(text) => {
            use flate2::write::GzEncoder;
            use std::io::Write;
            let mut encoder = GzEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(text.as_bytes()).unwrap();
            let compressed = encoder.finish().unwrap();
            axum::response::Response::builder()
                .header("content-type", "text/event-stream")
                .header("content-encoding", "gzip")
                .body(axum::body::Body::from(compressed))
                .unwrap()
        }
    }
}

fn json_response(path: &str, text: &str) -> axum::response::Response {
    let body = if path.ends_with("messages") {
        json!({"id":"msg-fixture","type":"message","role":"assistant","model":UPSTREAM,
            "content":[{"type":"text","text":text}],"stop_reason":"end_turn",
            "usage":{"input_tokens":1,"output_tokens":1}})
    } else if path.ends_with("chat/completions") {
        json!({"id":"chatcmpl-fixture","object":"chat.completion","model":UPSTREAM,
            "choices":[{"index":0,"message":{"role":"assistant","content":text},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}})
    } else if path.contains("/models/") {
        json!({"candidates":[{"content":{"role":"model","parts":[{"text":text}]},"finishReason":"STOP"}],
            "usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":1,"totalTokenCount":2}})
    } else {
        json!({"id":"resp-fixture","object":"response","model":UPSTREAM,"status":"completed",
            "output":[{"id":"msg-fixture","type":"message","role":"assistant","status":"completed",
            "content":[{"type":"output_text","text":text,"annotations":[]}]}],
            "usage":{"input_tokens":1,"output_tokens":1}})
    };
    let mut response = axum::Json(body).into_response();
    response
        .headers_mut()
        .insert("x-oneapi-request-id", "up-fixture".parse().unwrap());
    response
}

fn sse_chunks(
    text: String,
    cuts: Vec<usize>,
) -> impl futures::Stream<Item = Result<bytes::Bytes, std::io::Error>> {
    let mut pieces = Vec::new();
    let mut rest = text.as_str();
    for cut in cuts {
        let cut = cut.min(rest.len());
        let (head, tail) = rest.split_at(cut);
        pieces.push(head.to_owned());
        rest = tail;
    }
    pieces.push(rest.to_owned());
    futures::stream::iter(pieces)
        .enumerate()
        .map(|(index, piece)| async move {
            tokio::time::sleep(std::time::Duration::from_millis(5 * index as u64)).await;
            Ok(bytes::Bytes::from(piece))
        })
        .buffered(8)
}

// ---- 离线 remote / 夹具 ----

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
        Box::pin(async { Err(NewApiError::Network) })
    }
    fn create<'a>(&'a self, _: &'a Value) -> BoxFuture<'a, Result<(), NewApiError>> {
        Box::pin(async { Err(NewApiError::Network) })
    }
    fn keys<'a>(
        &'a self,
        _: &'a [i64],
    ) -> BoxFuture<'a, Result<BTreeMap<i64, String>, NewApiError>> {
        Box::pin(async { Err(NewApiError::Network) })
    }
}

struct Fixture {
    db: Arc<Database>,
    store: Arc<Memory>,
    proxy: ProxyServer,
    capture: Capture,
    mode: Mode,
    replacement: Replacement,
    #[allow(dead_code)]
    mock: tokio::task::JoinHandle<()>,
    origin: String,
}

impl Fixture {
    async fn new(mode: UpstreamMode, replacement: &str) -> Self {
        Self::build(mode, replacement.to_owned(), false).await
    }

    /// conversion = true 时把 Claude 的本地 provider 线改成 openai_responses
    ///（Claude→Responses 协议转换链路）。
    async fn build(mode: UpstreamMode, replacement: String, conversion: bool) -> Self {
        support::ensure_test_home();
        let _ = captured_logs();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mock_origin = format!("http://{}", listener.local_addr().unwrap());
        let capture = Capture::default();
        let mode = Arc::new(Mutex::new(mode));
        let replacement = Arc::new(Mutex::new(replacement));
        let app = axum::Router::new().fallback(upstream).with_state((
            capture.clone(),
            mode.clone(),
            replacement.clone(),
        ));
        let mock = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let db = Arc::new(Database::memory().unwrap());
        seed::init(&db).unwrap();
        let store = Arc::new(Memory::default());
        store
            .set("access-token", "synthetic-privacy-access")
            .unwrap();
        store
            .set("token:17", "synthetic-privacy-model-key")
            .unwrap();
        {
            let conn = routing_support::connection(&db);
            conn.execute(
                "INSERT INTO aicli_account(id,user_id,username,user_group,privacy_enabled,privacy_failure_mode)
                 VALUES(1,17,'fixture','fixture',1,'block')",
                [],
            )
            .unwrap();
            let groups = json!({"data":{"fixture":{"ratio":1}}});
            let pricing = json!({"auto_groups":["fixture"],"data":[{"model_name":UPSTREAM,
                "enable_groups":["fixture"],
                "supported_endpoint_types":["anthropic","openai","openai-response","gemini"]}]});
            for (kind, value) in [("groups", groups), ("pricing", pricing)] {
                conn.execute(
                    "INSERT INTO aicli_cache(kind,fetched_at,body_json) VALUES(?1,1,?2)",
                    rusqlite::params![kind, value.to_string()],
                )
                .unwrap();
            }
            conn.execute(
                r#"INSERT INTO aicli_managed_token VALUES(17,?1,'["fixture"]','fixture','masked',0)"#,
                [groups_hash(&["fixture".into()])],
            )
            .unwrap();
        }
        let mut runtime = AicliRuntime::with_remote(db.clone(), store.clone(), Arc::new(Offline));
        let profiles = ProfileStore::new(db.clone());
        let catalog = runtime.snapshot().catalog.clone().unwrap();
        let mut profile = profiles
            .copy_preset("recommended", "custom", "Custom", &catalog)
            .unwrap();
        profile.body.model_bindings.push(ModelBinding {
            model: UPSTREAM.into(),
            source_group: Some("fixture".into()),
            alias: Some(PUBLIC.into()),
        });
        profiles.save(&profile, &catalog).unwrap();
        routing_support::connection(&db)
            .execute("UPDATE aicli_account SET active_profile_id='custom'", [])
            .unwrap();
        let runtime = AicliRuntime::with_remote(db.clone(), store.clone(), Arc::new(Offline));
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
            if conversion && matches!(app, AppType::Claude | AppType::ClaudeDesktop) {
                // Claude 客户端走 Responses 线（协议转换链路）
                if let Some(meta) = provider.meta.as_mut() {
                    meta.api_format = Some("openai_responses".into());
                }
            }
            Some(provider)
        });
        let proxy = ProxyServer::new(
            ProxyConfig {
                listen_port: 0,
                ..Default::default()
            },
            db.clone(),
            None,
        )
        .with_test_runtime(runtime);
        let info = proxy.start().await.unwrap();
        Self {
            db,
            store,
            proxy,
            capture,
            mode,
            replacement,
            mock,
            origin: format!("http://127.0.0.1:{}", info.port),
        }
    }

    fn client(&self) -> reqwest::Client {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "x-api-key",
            cc_switch_lib::aicli::local_auth::token_for(&self.db, "codex")
                .unwrap()
                .parse()
                .unwrap(),
        );
        reqwest::Client::builder()
            .default_headers(headers)
            .no_proxy()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .unwrap()
    }

    fn gate(&self) -> Arc<cc_switch_lib::aicli::privacy::layer::PrivacyGate> {
        Arc::clone(self.proxy.test_state().aicli.privacy())
    }

    fn set_privacy(&self, enabled: bool, mode: &str, allow_media: bool) {
        routing_support::connection(&self.db)
            .execute(
                "UPDATE aicli_account SET privacy_enabled=?1,privacy_failure_mode=?2,allow_unscanned_media=?3",
                rusqlite::params![enabled, mode, allow_media],
            )
            .unwrap();
        cc_switch_lib::aicli::privacy::layer::bump_revision();
    }

    fn set_mode(&self, mode: UpstreamMode) {
        *self.mode.lock().unwrap() = mode;
    }

    fn set_replacement(&self, text: &str) {
        *self.replacement.lock().unwrap() = text.to_owned();
    }

    fn bodies(&self) -> Vec<Vec<u8>> {
        self.capture
            .lock()
            .unwrap()
            .iter()
            .map(|c| c.body.clone())
            .collect()
    }

    fn take_capture(&self, index: usize) -> Captured {
        self.capture.lock().unwrap()[index].clone()
    }

    async fn stop(self) {
        self.proxy.stop().await.unwrap();
        self.mock.abort();
    }
}

fn claude_alias(db: &Database) -> String {
    alias::encode(&AppType::Claude, PUBLIC, Some(db)).unwrap()
}

/// 从上游录制里抠出第一个 {{…}} 占位符。
fn first_placeholder(body: &str) -> String {
    let start = body.find("{{").expect("placeholder present");
    let end = body[start..].find("}}").unwrap() + start + 2;
    body[start..end].to_owned()
}

// ---- 1. 四协议非流式 ----

#[tokio::test]
async fn four_protocols_non_stream_masked_and_restored() {
    let email = email_value();
    let key = sk_key();
    let f = Fixture::new(UpstreamMode::JsonEcho, "ignored").await;
    let alias = claude_alias(&f.db);

    let claude = json!({"model": alias, "max_tokens": 64, "messages": [
        {"role":"user","content":[{"type":"text","text": format!("联系 {email}，密钥 {key}")}]}]});
    let out = f
        .client()
        .post(format!("{}/v1/messages", f.origin))
        .json(&claude)
        .send()
        .await
        .unwrap();
    assert_eq!(out.status(), 200);
    let text = out.text().await.unwrap();
    assert!(text.contains(&email), "restored for client: {text}");
    assert!(!text.contains("{{EMAIL_"), "{text}");
    assert!(!text.contains("{{"), "{text}");

    let out = f
        .client()
        .post(format!("{}/v1/chat/completions", f.origin))
        .json(&json!({"model": PUBLIC, "messages":[{"role":"user","content": format!("mail {email}")}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(out.status(), 200);
    assert!(out.text().await.unwrap().contains(&email));

    let out = f
        .client()
        .post(format!("{}/v1/responses", f.origin))
        .json(
            &json!({"model": PUBLIC, "input":[{"type":"message","role":"user",
            "content":[{"type":"input_text","text": format!("mail {email}")}]}]}),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(out.status(), 200);
    assert!(out.text().await.unwrap().contains(&email));

    let out = f
        .client()
        .post(format!(
            "{}/v1beta/models/{PUBLIC}:generateContent",
            f.origin
        ))
        .json(&json!({"contents":[{"role":"user","parts":[{"text": format!("mail {email}")}]}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(out.status(), 200);
    assert!(out.text().await.unwrap().contains(&email));

    // 上游收到的原始字节里没有任何敏感值，只有占位符
    for (index, body) in f.bodies().iter().enumerate() {
        let text = std::str::from_utf8(body).unwrap().to_owned();
        assert!(!text.contains(&email), "capture {index}: {text}");
        assert!(!text.contains(&key), "capture {index}: {text}");
        assert!(text.contains("{{"), "capture {index}: {text}");
    }
    // 应用日志不含敏感值
    for message in captured_logs().lock().unwrap().iter() {
        assert!(!message.contains(&email), "log leak: {message}");
        assert!(!message.contains(&key), "log leak: {message}");
    }
    f.stop().await;
}

// ---- 2. SSE：占位符拆在两个事件里 ----

#[tokio::test]
async fn sse_split_placeholder_restored() {
    let email = email_value();
    let f = Fixture::new(UpstreamMode::Json, "ok").await;
    // 请求方向遮盖同值，引擎值表里有完整占位符；上游录制里抠出来
    let _ = f
        .client()
        .post(format!("{}/v1/chat/completions", f.origin))
        .json(&json!({"model": PUBLIC, "messages":[{"role":"user","content": format!("mail {email}")}]}))
        .send()
        .await
        .unwrap();
    let placeholder =
        first_placeholder(std::str::from_utf8(f.take_capture(0).body.as_slice()).unwrap());
    // 同一夹具切到 SSE：占位符拆在两个 content delta 里下发
    let (head, tail) = placeholder.split_at(placeholder.len() / 2);
    let sse = format!(
        "data: {{\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"见 {head}\"}}}}]}}\n\n\
         data: {{\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{tail} 吧\"}}}}]}}\n\n\
         data: [DONE]\n\n"
    );
    f.set_mode(UpstreamMode::Sse(sse, vec![40, 90, 200]));
    let out = f
        .client()
        .post(format!("{}/v1/chat/completions", f.origin))
        .json(
            &json!({"model": PUBLIC, "stream": true, "messages":[{"role":"user","content":"hi"}]}),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(out.status(), 200);
    let full = out.text().await.unwrap();
    assert!(full.contains(&email), "joined and restored: {full}");
    assert!(!full.contains("{{EMAIL_"), "{full}");
    f.stop().await;
}

// ---- 3. 关闭隐私逐字节相同 ----

#[tokio::test]
async fn disabled_privacy_body_is_byte_identical() {
    let f = Fixture::new(UpstreamMode::Json, "ok").await;
    f.set_privacy(false, "block", false);
    let body = json!({"model": claude_alias(&f.db), "max_tokens": 8, "messages": [
        {"role":"user","content":"plain hello 中文 😀"}]});
    let first = f
        .client()
        .post(format!("{}/v1/messages", f.origin))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), 200);
    let baseline = f.bodies().remove(0);

    // 同一请求再发：逐字节一致（关闭 = 纯透传，等价于未加中间件）
    let _ = f
        .client()
        .post(format!("{}/v1/messages", f.origin))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(f.bodies()[1], baseline, "repeat identical");

    // 开启隐私但正文无敏感值：上游 body 仍逐字节相同
    f.set_privacy(true, "block", false);
    let _ = f
        .client()
        .post(format!("{}/v1/messages", f.origin))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(f.bodies()[2], baseline, "enabled with clean body identical");
    f.stop().await;
}

// ---- 4. Claude→Responses 协议转换 ----

#[tokio::test]
async fn claude_to_responses_conversion_keeps_placeholders() {
    let email = email_value();
    let f = Fixture::build(UpstreamMode::JsonEcho, "ignored".into(), true).await;
    let body = json!({"model": claude_alias(&f.db), "max_tokens": 64, "messages": [
        {"role":"user","content":[{"type":"text","text": format!("mail {email}")}]}]});
    let out = f
        .client()
        .post(format!("{}/v1/messages", f.origin))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(out.status(), 200);
    // 上游收到 Responses 端点的请求，且含占位符、不含原文
    let captures = f.capture.lock().unwrap();
    let responses_capture = captures
        .iter()
        .find(|c| c.path.contains("responses"))
        .expect("conversion hit /responses");
    let text = std::str::from_utf8(responses_capture.body.as_slice())
        .unwrap()
        .to_owned();
    assert!(!text.contains(&email), "{text}");
    assert!(text.contains("{{EMAIL_"), "{text}");
    drop(captures);
    // 客户端（Claude 协议）拿到还原后的文本
    let text = out.text().await.unwrap();
    assert!(text.contains(&email), "client restored: {text}");
    f.stop().await;
}

// ---- 5. 重启后占位符不变（提示缓存前缀不变） ----

#[tokio::test]
async fn restart_produces_identical_placeholders() {
    let email = email_value();
    let f = Fixture::new(UpstreamMode::Json, "ok").await;
    let body =
        json!({"model": PUBLIC, "messages":[{"role":"user","content": format!("mail {email}")}]});
    let _ = f
        .client()
        .post(format!("{}/v1/chat/completions", f.origin))
        .json(&body)
        .send()
        .await
        .unwrap();
    let first = f.bodies().remove(0);
    let db = f.db.clone();
    let store = f.store.clone();
    f.stop().await;

    // 同一 db + 同一 store（同一把密钥）：新运行时 + 新代理（重启）
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mock_origin = format!("http://{}", listener.local_addr().unwrap());
    let capture = Capture::default();
    let mode = Arc::new(Mutex::new(UpstreamMode::Json));
    let replacement = Arc::new(Mutex::new("ok".to_owned()));
    let app = axum::Router::new().fallback(upstream).with_state((
        capture.clone(),
        mode.clone(),
        replacement.clone(),
    ));
    let mock = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let runtime = AicliRuntime::with_remote(db.clone(), store, Arc::new(Offline));
    runtime.set_local_test_provider_source(move |app, line, wire| {
        let mut provider = seed::clone_for(app, line, wire).unwrap();
        provider.settings_config["base_url"] = json!(mock_origin);
        Some(provider)
    });
    let proxy = ProxyServer::new(
        ProxyConfig {
            listen_port: 0,
            ..Default::default()
        },
        db,
        None,
    )
    .with_test_runtime(runtime);
    let info = proxy.start().await.unwrap();
    let state = proxy.test_state();
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "x-api-key",
        cc_switch_lib::aicli::local_auth::token_for(&state.db, "codex")
            .unwrap()
            .parse()
            .unwrap(),
    );
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .no_proxy()
        .build()
        .unwrap();
    let _ = client
        .post(format!(
            "http://127.0.0.1:{}/v1/chat/completions",
            info.port
        ))
        .json(&body)
        .send()
        .await
        .unwrap();
    let second = capture.lock().unwrap().remove(0).body;
    proxy.stop().await.unwrap();
    mock.abort();
    assert_eq!(first, second, "placeholders stable across restart");
}

// ---- 6. 故障注入 3 位置 × 2 模式 ----

#[tokio::test]
async fn fault_injection_matrix() {
    let email = email_value();
    let request =
        json!({"model": PUBLIC, "messages":[{"role":"user","content": format!("mail {email}")}]});

    // 6a. 请求遮盖失败
    let f = Fixture::new(UpstreamMode::Json, "ok").await;
    f.gate().inject_failure(FAULT_MASK);
    let out = f
        .client()
        .post(format!("{}/v1/chat/completions", f.origin))
        .json(&request)
        .send()
        .await
        .unwrap();
    let body = out.text().await.unwrap();
    assert!(
        body.contains("privacy_engine_failed"),
        "block mode blocks: {body}"
    );
    assert!(f.bodies().is_empty(), "nothing sent upstream");
    f.gate().inject_failure(0);
    f.stop().await;

    let f = Fixture::new(UpstreamMode::Json, "ok").await;
    f.set_privacy(true, "pass", false);
    f.gate().inject_failure(FAULT_MASK);
    let out = f
        .client()
        .post(format!("{}/v1/chat/completions", f.origin))
        .json(&request)
        .send()
        .await
        .unwrap();
    assert_eq!(out.status(), 200);
    let captured = f.bodies().remove(0);
    assert!(
        std::str::from_utf8(captured.as_slice())
            .unwrap()
            .to_owned()
            .contains(&email),
        "pass mode forwards original"
    );
    assert_eq!(f.gate().stats().degraded, 1);
    f.gate().inject_failure(0);
    f.stop().await;

    // 6b. 非流式还原失败
    let f = Fixture::new(UpstreamMode::Json, "ok").await;
    f.gate().inject_failure(FAULT_RESTORE_JSON);
    let out = f
        .client()
        .post(format!("{}/v1/chat/completions", f.origin))
        .json(&request)
        .send()
        .await
        .unwrap();
    let body = out.text().await.unwrap();
    assert!(
        body.contains("privacy_engine_failed"),
        "block mode restore blocked: {body}"
    );
    f.gate().inject_failure(0);
    f.stop().await;

    let f = Fixture::new(UpstreamMode::JsonEcho, "ignored").await;
    f.set_privacy(true, "pass", false);
    f.gate().inject_failure(FAULT_RESTORE_JSON);
    let out = f
        .client()
        .post(format!("{}/v1/chat/completions", f.origin))
        .json(&request)
        .send()
        .await
        .unwrap();
    assert_eq!(out.status(), 200);
    let body = out.text().await.unwrap();
    assert!(body.contains("{{"), "pass mode leaves placeholders: {body}");
    assert!(!body.contains(&email));
    assert_eq!(f.gate().stats().degraded, 1);
    f.gate().inject_failure(0);
    f.stop().await;

    // 6c. 流式还原失败
    let f = Fixture::new(UpstreamMode::Json, "ok").await;
    let _ = f
        .client()
        .post(format!("{}/v1/chat/completions", f.origin))
        .json(&request)
        .send()
        .await
        .unwrap();
    let placeholder =
        first_placeholder(std::str::from_utf8(f.take_capture(0).body.as_slice()).unwrap());
    let sse = format!(
        "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{}\"}}}}]}}\n\ndata: [DONE]\n\n",
        placeholder
    );
    let stream_req =
        json!({"model": PUBLIC, "stream": true, "messages":[{"role":"user","content":"hi"}]});
    // block 模式：流立即结束，不透传占位符文本
    f.set_mode(UpstreamMode::Sse(sse.clone(), vec![60]));
    f.gate().inject_failure(FAULT_STREAM);
    let out = f
        .client()
        .post(format!("{}/v1/chat/completions", f.origin))
        .json(&stream_req)
        .send()
        .await
        .unwrap();
    assert_eq!(out.status(), 200);
    let text = out.text().await.unwrap();
    assert!(!text.contains("{{EMAIL_"), "block mode ends stream: {text}");
    f.gate().inject_failure(0);
    // pass 模式：原样透传（占位符），不挂起
    let degraded_before = f.gate().stats().degraded;
    f.set_mode(UpstreamMode::Sse(sse, vec![60]));
    f.set_privacy(true, "pass", false);
    f.gate().inject_failure(FAULT_STREAM);
    let out = f
        .client()
        .post(format!("{}/v1/chat/completions", f.origin))
        .json(&stream_req)
        .send()
        .await
        .unwrap();
    assert_eq!(out.status(), 200);
    let text = out.text().await.unwrap();
    assert!(
        text.contains("{{EMAIL_"),
        "pass mode raw passthrough: {text}"
    );
    assert_eq!(f.gate().stats().degraded, degraded_before + 1);
    f.gate().inject_failure(0);
    f.stop().await;
}

// ---- 7. 层顺序：未过鉴权不读 body ----

#[tokio::test]
async fn unauthorized_requests_skip_privacy_layer() {
    let f = Fixture::new(UpstreamMode::Json, "ok").await;
    let plain = reqwest::Client::builder().no_proxy().build().unwrap();
    let before = f.gate().stats().bodies_read;
    let out = plain
        .post(format!("{}/v1/messages", f.origin))
        .header("x-api-key", "wrong-token")
        .json(&json!({"model": PUBLIC, "messages": []}))
        .send()
        .await
        .unwrap();
    assert_eq!(out.status(), 401);
    assert_eq!(
        f.gate().stats().bodies_read,
        before,
        "privacy layer did not read body"
    );
    assert!(f.bodies().is_empty());
    f.stop().await;
}

// ---- 8. 流式响应进行中，另一个请求能完成遮盖（隐私层不跨流持锁） ----
//
// 全链路（含 forwarder/usage 记录）对流式响应与并发请求的串行化是既有
// 行为：关闭隐私后同样的双请求同样阻塞（诊断记录见 E-06.txt 偏差）。
// 本测试直接针对隐私层 [验收] 项：只挂隐私中间件的 app 上，一个 SSE
// 流挂着时，另一个请求能拿到引擎写锁完成遮盖。

#[tokio::test]
async fn masking_works_while_stream_is_open() {
    use axum::routing::post;

    let release = Arc::new(Notify::new());
    let db = Arc::new(Database::memory().unwrap());
    seed::init(&db).unwrap();
    let store = Arc::new(Memory::default());
    let runtime = AicliRuntime::with_remote(db.clone(), store, Arc::new(Offline));
    let proxy = ProxyServer::new(
        ProxyConfig {
            listen_port: 0,
            ..Default::default()
        },
        db.clone(),
        None,
    )
    .with_test_runtime(runtime);
    let state = proxy.test_state();
    routing_support::connection(&db)
        .execute(
            "INSERT INTO aicli_account(id,privacy_enabled,privacy_failure_mode)
             VALUES(1,1,'block')
             ON CONFLICT(id) DO UPDATE SET privacy_enabled=1, privacy_failure_mode='block'",
            [],
        )
        .unwrap();
    let first_release = release.clone();
    let sse_handler = move || async move {
        let stream = async_stream::stream! {
            yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(
                b"data: {\"choices\":[{\"delta\":{\"content\":\"first\"}}]}

",
            ));
            first_release.notified().await;
            yield Ok(bytes::Bytes::from_static(b"data: [DONE]

"));
        };
        axum::response::Response::builder()
            .header("content-type", "text/event-stream")
            .body(axum::body::Body::from_stream(stream))
            .unwrap()
    };
    let app = axum::Router::new()
        .route("/v1/messages", post(sse_handler))
        .route(
            "/v1/chat/completions",
            post(|| async { axum::Json(json!({"ok": true})).into_response() }),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            cc_switch_lib::aicli::privacy::layer::privacy_middleware,
        ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let email = email_value();
    let stream_client = reqwest::Client::builder().no_proxy().build().unwrap();
    let stream_origin = origin.clone();
    let holder = tokio::spawn(async move {
        let out = stream_client
            .post(format!("{stream_origin}/v1/messages"))
            .header("content-type", "application/json")
            .body(r#"{"messages":[{"content":"hi"}]}"#.to_owned())
            .send()
            .await
            .unwrap();
        let _ = out.text().await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    // 流挂着：另一个请求要能完成遮盖（写锁不被流的长读锁挡住）
    let other = reqwest::Client::builder().no_proxy().build().unwrap();
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        other
            .post(format!("{origin}/v1/chat/completions"))
            .json(&json!({"messages":[{"role":"user","content": format!("mail {email}")}]}))
            .send(),
    )
    .await
    .expect("second request completes while stream is open")
    .unwrap();
    assert_eq!(out.status(), 200);
    let _ = out.text().await.unwrap();
    // 遮盖确实完成（引擎写锁在流挂着时可用）：masked_values 增加
    assert!(
        state.aicli.privacy().stats().masked_values >= 1,
        "masking completed while stream open"
    );
    release.notify_one();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(10), holder).await;
    server.abort();
}

// ---- 9. 媒体：接口不支持 / data: 阻断与放行 ----

#[tokio::test]
async fn media_paths_block_and_pass() {
    let f = Fixture::new(UpstreamMode::Json, "ok").await;
    let out = f
        .client()
        .post(format!("{}/v1/images/generations", f.origin))
        .json(&json!({"model": PUBLIC, "prompt":"a cat"}))
        .send()
        .await
        .unwrap();
    let body = out.text().await.unwrap();
    assert!(body.contains("privacy_media_unsupported"), "{body}");

    f.set_privacy(true, "pass", false);
    let out = f
        .client()
        .post(format!("{}/v1/images/generations", f.origin))
        .json(&json!({"model": PUBLIC, "prompt":"a cat"}))
        .send()
        .await
        .unwrap();
    assert_eq!(out.status(), 200);

    f.set_privacy(true, "block", false);
    let email = email_value();
    let with_media = json!({"model": PUBLIC, "messages":[{"role":"user","content":[
        {"type":"text","text": format!("mail {email}")},
        {"type":"image_url","image_url":{"url":"data:image/png;base64,aGVsbG8="}}]}]});
    let out = f
        .client()
        .post(format!("{}/v1/chat/completions", f.origin))
        .json(&with_media)
        .send()
        .await
        .unwrap();
    let body = out.text().await.unwrap();
    assert!(body.contains("privacy_media_blocked"), "{body}");

    f.set_privacy(true, "block", true);
    let out = f
        .client()
        .post(format!("{}/v1/chat/completions", f.origin))
        .json(&with_media)
        .send()
        .await
        .unwrap();
    assert_eq!(out.status(), 200);
    let captured = f.bodies().pop().unwrap();
    let text = std::str::from_utf8(captured.as_slice()).unwrap().to_owned();
    assert!(!text.contains(&email), "{text}");
    assert!(text.contains("data:image/png;base64,aGVsbG8="), "{text}");
    // 计数 = 图片接口 pass 模式放行 1 + data: 媒体放行 1
    assert_eq!(f.gate().stats().media_passed, 2);
    f.stop().await;
}

// ---- 10. 压缩流 fail-closed ----

#[tokio::test]
async fn compressed_stream_is_fail_closed() {
    let sse =
        "data: {\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n\ndata: [DONE]\n\n".to_owned();
    let f = Fixture::new(UpstreamMode::SseCompressed(sse), "ignored").await;
    let out = f
        .client()
        .post(format!("{}/v1/chat/completions", f.origin))
        .json(
            &json!({"model": PUBLIC, "stream": true, "messages":[{"role":"user","content":"hi"}]}),
        )
        .send()
        .await
        .unwrap();
    let body = out.text().await.unwrap();
    assert!(body.contains("privacy_compressed_stream"), "{body}");
    f.stop().await;
}

// ---- 11. 关闭隐私时：完全不读 body（层为纯透传） ----

#[tokio::test]
async fn disabled_privacy_never_reads_bodies() {
    let f = Fixture::new(UpstreamMode::Json, "ok").await;
    f.set_privacy(false, "block", false);
    let before = f.gate().stats().bodies_read;
    let _ = f
        .client()
        .post(format!("{}/v1/messages", f.origin))
        .json(
            &json!({"model": claude_alias(&f.db), "max_tokens": 8, "messages": [
            {"role":"user","content":"hi"}]}),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(
        f.gate().stats().bodies_read,
        before,
        "no body read when off"
    );
    f.stop().await;
}
