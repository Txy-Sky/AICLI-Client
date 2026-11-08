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
    /// gzip 压缩 SSE（fail-closed 测试用）。
    SseCompressed(String),
    /// 按请求决定：`"stream":true` 的请求回首个 SSE 事件后挂起，等 release
    /// 再结束；其他请求立即回 JSON（全链路并发测试用，同一 mock 同时服务两类请求）。
    HeldStreamOrJson(Arc<Notify>),
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
        UpstreamMode::HeldStreamOrJson(release) => {
            let streaming = serde_json::from_slice::<Value>(&bytes)
                .ok()
                .and_then(|body| body.get("stream").and_then(Value::as_bool))
                .unwrap_or(false);
            if !streaming {
                return json_response(&path, "ok");
            }
            let (head, tail) = held_sse(&path);
            let stream = async_stream::stream! {
                yield Ok::<_, std::io::Error>(bytes::Bytes::from(head));
                release.notified().await;
                yield Ok(bytes::Bytes::from(tail));
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

/// 全链路并发测试的 SSE：首段（立即发出）与尾段（release 后发出）。
fn held_sse(path: &str) -> (String, String) {
    if path.ends_with("messages") {
        let head = [
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg-held\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"upstream-fixture\",\"content\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"first\"}}\n\n",
        ]
        .concat();
        let tail = [
            "event: content_block_stop\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        ]
        .concat();
        (head, tail)
    } else {
        (
            "data: {\"id\":\"chatcmpl-held\",\"object\":\"chat.completion.chunk\",\"model\":\"upstream-fixture\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"first\"}}]}\n\n".to_owned(),
            "data: {\"id\":\"chatcmpl-held\",\"object\":\"chat.completion.chunk\",\"model\":\"upstream-fixture\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".to_owned(),
        )
    }
}

fn sse_chunks(
    text: String,
    cuts: Vec<usize>,
) -> impl futures::Stream<Item = Result<bytes::Bytes, std::io::Error>> {
    let mut pieces = Vec::new();
    let mut rest = text.into_bytes();
    for cut in cuts {
        let cut = cut.min(rest.len());
        let tail = rest.split_off(cut);
        pieces.push(bytes::Bytes::from(rest));
        rest = tail;
    }
    pieces.push(bytes::Bytes::from(rest));
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
        let runtime = AicliRuntime::with_remote(db.clone(), store.clone(), Arc::new(Offline));
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
        let mock_origin_for_closure = mock_origin.clone();
        runtime.set_local_test_provider_source(move |app, line, wire| {
            let mut provider = seed::clone_for(app, line, wire).unwrap();
            match app {
                AppType::Claude | AppType::ClaudeDesktop => {
                    provider.settings_config["env"]["ANTHROPIC_BASE_URL"] =
                        json!(mock_origin_for_closure)
                }
                AppType::Gemini => {
                    provider.settings_config["env"]["GOOGLE_GEMINI_BASE_URL"] =
                        json!(mock_origin_for_closure)
                }
                _ => provider.settings_config["base_url"] = json!(mock_origin_for_closure),
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
    // block 模式：返回错误断开连接（或无占位符的截断流），客户端能看出失败
    f.set_mode(UpstreamMode::Sse(sse.clone(), vec![60]));
    f.gate().inject_failure(FAULT_STREAM);
    match f
        .client()
        .post(format!("{}/v1/chat/completions", f.origin))
        .json(&stream_req)
        .send()
        .await
    {
        Err(_) => { /* 连接中断：失败可见 ✓ */ }
        Ok(out) => match out.text().await {
            Err(_) => { /* body 中断：失败可见 ✓ */ }
            Ok(text) => {
                assert!(
                    !text.contains("{{EMAIL_"),
                    "no placeholder forwarded: {text}"
                )
            }
        },
    }
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
// 只挂隐私中间件的 mini app：直接验证隐私层不跨流持锁（一个 SSE 流
// 挂着时另一请求能拿到引擎写锁完成遮盖）。真实代理上的全链路并发见
// 第 17 个测试。

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

// ---- 12. 媒体形态补齐：Claude source base64（image/document）与 Gemini
//      inlineData —— block 未放行 → 422 + 上游零新增捕获；pass 放行 ----

#[tokio::test]
async fn claude_and_gemini_media_shapes_block_and_pass() {
    let f = Fixture::new(UpstreamMode::Json, "ok").await;
    let email = email_value();
    let claude_image = json!({"model": claude_alias(&f.db), "max_tokens": 8, "messages": [
        {"role":"user","content":[
            {"type":"image","source":{"type":"base64","media_type":"image/png","data":"aGVsbG8="}},
            {"type":"text","text": format!("mail {email}")}]}]});
    let claude_document = json!({"model": claude_alias(&f.db), "max_tokens": 8, "messages": [
        {"role":"user","content":[
            {"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"ZG9j"}},
            {"type":"text","text": format!("mail {email}")}]}]});
    let gemini_inline = json!({"contents":[{"role":"user","parts":[
        {"inlineData":{"mimeType":"image/png","data":"aGVsbG8="}},
        {"text": format!("mail {email}")}]}]});

    for (name, path, body) in [
        ("claude-image", "/v1/messages", &claude_image),
        ("claude-document", "/v1/messages", &claude_document),
        (
            "gemini-inline",
            &format!("/v1beta/models/{PUBLIC}:generateContent"),
            &gemini_inline,
        ),
    ] {
        // block 且未放行：422 + 上游零新增捕获
        let before = f.capture.lock().unwrap().len();
        let out = f
            .client()
            .post(format!("{}{path}", f.origin))
            .json(body)
            .send()
            .await
            .unwrap();
        let status = out.status();
        let text = out.text().await.unwrap();
        assert_eq!(status, 422, "{name}: {text}");
        assert!(text.contains("privacy_media_blocked"), "{name}: {text}");
        assert_eq!(
            f.capture.lock().unwrap().len(),
            before,
            "{name}: nothing forwarded"
        );
        // pass 模式：放行（文本遮盖、data 原样）并计数
        f.set_privacy(true, "pass", false);
        let out = f
            .client()
            .post(format!("{}{path}", f.origin))
            .json(body)
            .send()
            .await
            .unwrap();
        assert_eq!(out.status(), 200, "{name}");
        let captured = f.bodies().pop().unwrap();
        let text = std::str::from_utf8(captured.as_slice()).unwrap().to_owned();
        assert!(!text.contains(&email), "{name}: {text}");
        f.set_privacy(true, "block", false);
    }
    f.stop().await;
}

// ---- 13. pass 模式 mask 失败：发解压后的明文（不是压缩字节） ----

#[tokio::test]
async fn pass_mode_mask_failure_forwards_decompressed_plaintext() {
    use flate2::write::GzEncoder;
    use std::io::Write;

    let f = Fixture::new(UpstreamMode::Json, "ok").await;
    f.set_privacy(true, "pass", false);
    f.gate().inject_failure(FAULT_MASK);
    let email = email_value();
    let body =
        json!({"model": PUBLIC, "messages":[{"role":"user","content": format!("mail {email}")}]});
    let mut encoder = GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(body.to_string().as_bytes()).unwrap();
    let compressed = encoder.finish().unwrap();
    let out = f
        .client()
        .post(format!("{}/v1/chat/completions", f.origin))
        .header("content-encoding", "gzip")
        .header("content-type", "application/json")
        .body(compressed)
        .send()
        .await
        .unwrap();
    assert_eq!(out.status(), 200);
    f.gate().inject_failure(0);
    // 上游收到**可解析的明文**（原文放行；content-encoding 已删）
    let captured = f.bodies().pop().unwrap();
    let text = std::str::from_utf8(captured.as_slice()).unwrap().to_owned();
    let parsed: Value = serde_json::from_str(&text).expect("upstream got plaintext JSON");
    assert!(parsed["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains(&email));
    // 且发往上游的请求没有 content-encoding 头
    let headers = f.capture.lock().unwrap().last().unwrap().headers.clone();
    assert!(
        !headers.contains_key("content-encoding"),
        "header removed: {headers:?}"
    );
    f.stop().await;
}

// ---- 14. 流式 pass 模式第 N 块故障：缓存的原始字节先放出，SSE 帧不重不漏 ----

#[tokio::test]
async fn stream_pass_mode_fault_at_nth_chunk_keeps_frames_intact() {
    let f = Fixture::new(UpstreamMode::Json, "ok").await;
    let email = email_value();
    // 请求遮盖同值，拿到占位符
    let request =
        json!({"model": PUBLIC, "messages":[{"role":"user","content": format!("mail {email}")}]});
    let _ = f
        .client()
        .post(format!("{}/v1/chat/completions", f.origin))
        .json(&request)
        .send()
        .await
        .unwrap();
    let placeholder = first_placeholder(
        &std::str::from_utf8(f.take_capture(0).body.as_slice())
            .unwrap()
            .to_owned(),
    );
    let (head, tail) = placeholder.split_at(placeholder.len() / 2);
    // 三块：块 1 = 完整事件（结尾半个占位符 → 被扣留），块 2 触发故障，块 3 = 其余
    let ev1 = format!(
        "data: {{\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"见 {head}\"}}}}]}}

"
    );
    let ev2 = format!(
        "data: {{\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{tail} 吧\"}}}}]}}

"
    );
    let sse = format!(
        "{ev1}{ev2}data: [DONE]

"
    );
    f.set_mode(UpstreamMode::Sse(
        sse.clone(),
        vec![ev1.len(), ev2.len(), 6],
    ));
    // pass 模式 + 故障在第 2 块
    f.set_privacy(true, "pass", false);
    f.gate().set_stream_fault_event(2);
    f.gate().inject_failure(FAULT_STREAM);
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
    let received = out.text().await.unwrap();
    f.gate().inject_failure(0);
    // 块 1 的事件被扣留（未输出）→ 故障后随缓存放出；之后原样透传：
    // 客户端收到的 SSE 与上游原文逐字节一致（每个事件完整、不重不漏）
    assert_eq!(received, sse, "frames intact, no dup/loss");
}

// ---- 15. 流式 block 模式第 N 块故障：失败可见、不透传占位符 ----

#[tokio::test]
async fn stream_block_mode_fault_at_nth_chunk_is_visible_failure() {
    let f = Fixture::new(UpstreamMode::Json, "ok").await;
    let email = email_value();
    let request =
        json!({"model": PUBLIC, "messages":[{"role":"user","content": format!("mail {email}")}]});
    let _ = f
        .client()
        .post(format!("{}/v1/chat/completions", f.origin))
        .json(&request)
        .send()
        .await
        .unwrap();
    let placeholder = first_placeholder(
        &std::str::from_utf8(f.take_capture(0).body.as_slice())
            .unwrap()
            .to_owned(),
    );
    let (head, tail) = placeholder.split_at(placeholder.len() / 2);
    let ev1 = format!(
        "data: {{\"choices\":[{{\"delta\":{{\"content\":\"见 {head}\"}}}}]}}

"
    );
    let ev2 = format!(
        "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{tail}\"}}}}]}}

"
    );
    let sse = format!(
        "{ev1}{ev2}data: [DONE]

"
    );
    f.set_mode(UpstreamMode::Sse(sse, vec![ev1.len(), ev2.len(), 6]));
    f.gate().set_stream_fault_event(2);
    f.gate().inject_failure(FAULT_STREAM);
    match f
        .client()
        .post(format!("{}/v1/chat/completions", f.origin))
        .json(
            &json!({"model": PUBLIC, "stream": true, "messages":[{"role":"user","content":"hi"}]}),
        )
        .send()
        .await
    {
        Err(_) => { /* 连接中断 ✓ */ }
        Ok(out) => match out.text().await {
            Err(_) => { /* body 中断 ✓ */ }
            // 隐私优先：被扣留的 ev1 与出错后的上游字节都不放出
            Ok(text) => assert!(!text.contains(head), "nothing after the fault: {text}"),
        },
    }
    f.gate().inject_failure(0);
    // block 模式不计入 degraded（单独计数 engine_blocked）
    let stats = f.gate().stats();
    assert_eq!(stats.degraded, 0, "block failures are not degradations");
    assert!(stats.engine_blocked >= 1);
    f.stop().await;
}

// ---- 16. 事件级注入：块内含完整事件 + 出错事件 + 下一事件开头，
//      中文/emoji 中间切块；字节 = 已还原事件 + 其后原始字节，无 U+FFFD ----

#[tokio::test]
async fn stream_fault_mid_event_keeps_restored_prefix_and_raw_suffix() {
    let f = Fixture::new(UpstreamMode::Json, "ok").await;
    let email = email_value();
    let request =
        json!({"model": PUBLIC, "messages":[{"role":"user","content": format!("mail {email}")}]});
    let _ = f
        .client()
        .post(format!("{}/v1/chat/completions", f.origin))
        .json(&request)
        .send()
        .await
        .unwrap();
    let placeholder = first_placeholder(
        &std::str::from_utf8(f.take_capture(0).body.as_slice())
            .unwrap()
            .to_owned(),
    );
    // ev1：完整占位符（会被还原并输出），文本带 emoji
    let ev1 = format!(
        "data: {{\"id\":\"a\",\"object\":\"chat.completion.chunk\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"😀 见 {placeholder}\"}}}}]}}

"
    );
    // ev2：出错事件（还原时注入 panic）
    let ev2 = format!(
        "data: {{\"id\":\"b\",\"object\":\"chat.completion.chunk\",\"choices\":{{\"index\":0,\"delta\":{{\"content\":\"第二段 中文\"}}}}]}}

"
    );
    // ev3：只发开头（留在 buf，失败瞬间以原始字节放出；尾块后续透传）
    let ev3_full = "data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"收尾\"}}]}

data: [DONE]

";
    let stream = format!("{ev1}{ev2}{ev3_full}");
    // 块切点：块 1 在 ev1 的 emoji 字节中间切开
    let emoji_at = ev1.find("😀").unwrap();
    let cut1 = emoji_at + 1; // emoji 是 4 字节 UTF-8，切在中间
                             // 块 2 = ev1 余下 + ev2 全部 + ev3 的前 1/3（含中文字节中间切点）
    let ev3_start = ev1.len() + ev2.len();
    let cut2 = ev3_start + ev3_full.find("收尾").unwrap() + 1; // 「收」是 3 字节，切中间
    let cuts = vec![cut1, cut2];
    f.set_mode(UpstreamMode::Sse(stream.clone(), cuts));
    f.set_privacy(true, "pass", false);
    f.gate().set_stream_fault_event(2); // ev2 还原时 panic
    f.gate().inject_failure(FAULT_STREAM);
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
    let bytes = out.bytes().await.unwrap();
    f.gate().inject_failure(0);
    // 期望：ev1（已还原：占位符→原值）+ 其后（ev2 起）的原始字节
    let expected = format!(
        "data: {{\"id\":\"a\",\"object\":\"chat.completion.chunk\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"😀 见 {email}\"}}}}]}}

{ev2}{ev3_full}"
    );
    assert_eq!(
        bytes.as_ref(),
        expected.as_bytes(),
        "restored prefix + raw suffix, byte-exact"
    );
    // 没有 U+FFFD（有损转换会出现在多字节被切坏时）
    assert!(
        !bytes.windows(3).any(|w| w == [0xEF, 0xBF, 0xBD]),
        "no U+FFFD"
    );
    // 降级计数一次
    assert_eq!(f.gate().stats().degraded, 1);
    f.stop().await;
}

// ---- 17. 全链路并发：流式响应进行中，同一上游的另一个请求能完成 ----
//
// 真实代理 + 真实 mock，多线程运行时。mock 按请求决定回挂起的 SSE 还是
// 立即回 JSON。E-06 验收时的「全链路串行化」诊断，最可能是因为 mock 用的是全局 mode：第二个
// 请求同样拿到挂起的 SSE，响应体自然要等 release，并非代理串行。
// 覆盖 OpenAI chat（共享连接池）与 Claude messages（hyper 原始连接）两条
// 转发路径 × 隐私关闭/开启。

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn second_request_completes_while_stream_is_open_full_chain() {
    let email = email_value();
    for (label, privacy) in [("off", false), ("on", true)] {
        for claude in [false, true] {
            let release = Arc::new(Notify::new());
            let f = Fixture::new(UpstreamMode::HeldStreamOrJson(release.clone()), "ok").await;
            f.set_privacy(privacy, "block", false);
            let (path, model) = if claude {
                ("/v1/messages", claude_alias(&f.db))
            } else {
                ("/v1/chat/completions", PUBLIC.to_owned())
            };
            let body = |stream: bool, text: &str| {
                json!({"model": model, "max_tokens": 8, "stream": stream,
                    "messages":[{"role":"user","content": text}]})
            };
            let case = format!("privacy={label} path={path}");

            // 1. 流式请求：拿到响应头与首个事件后保持打开
            let started = std::time::Instant::now();
            let stream_response = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                f.client()
                    .post(format!("{}{path}", f.origin))
                    .json(&body(true, "hold"))
                    .send(),
            )
            .await
            .unwrap_or_else(|_| panic!("{case}: stream headers"))
            .unwrap();
            assert_eq!(stream_response.status(), 200, "{case}");
            let mut stream = stream_response.bytes_stream();
            let first = tokio::time::timeout(std::time::Duration::from_secs(10), stream.next())
                .await
                .unwrap_or_else(|_| panic!("{case}: first stream chunk"));
            assert!(first.is_some(), "{case}: stream open");
            eprintln!("{case}: stream open after {:?}", started.elapsed());

            // 2. 流挂着：另一个非流式请求必须完成
            let second_started = std::time::Instant::now();
            let second = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                let out = f
                    .client()
                    .post(format!("{}{path}", f.origin))
                    .json(&body(false, &format!("mail {email}")))
                    .send()
                    .await
                    .unwrap();
                let status = out.status();
                (status, out.text().await.unwrap())
            })
            .await;
            eprintln!(
                "{case}: second request {} after {:?}",
                if second.is_ok() {
                    "finished"
                } else {
                    "TIMED OUT"
                },
                second_started.elapsed()
            );
            let (status, text) =
                second.unwrap_or_else(|_| panic!("{case}: second request blocked"));
            assert_eq!(status, 200, "{case}: {text}");
            if privacy {
                // 上游收到的是占位符，客户端拿到的是还原后的 JSON
                let captured = f.bodies();
                let last = std::str::from_utf8(captured.last().unwrap())
                    .unwrap()
                    .to_owned();
                assert!(!last.contains(&email), "{case}: masked upstream");
                assert!(f.gate().stats().masked_values >= 1, "{case}");
            }

            // 3. 放行流式尾段，流正常结束
            release.notify_one();
            let rest = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                while let Some(chunk) = stream.next().await {
                    chunk.unwrap();
                }
            })
            .await;
            assert!(rest.is_ok(), "{case}: stream finished after release");
            f.stop().await;
        }
    }
}

// ---- 18. E-08 中文姓名开关经真实代理生效（关 → 开 → 关） ----

#[tokio::test]
async fn cjk_name_switch_applies_through_proxy() {
    let f = Fixture::new(UpstreamMode::Json, "ok").await;
    f.set_privacy(true, "block", false);
    let name = ["陈", "芳宇"].concat();
    let body = json!({"model": claude_alias(&f.db), "max_tokens": 8, "messages": [
        {"role":"user","content": format!("联系人：{name}，请尽快确认")}]});
    let set_switch = |on: bool| {
        routing_support::connection(&f.db)
            .execute(
                "UPDATE aicli_account SET privacy_detect_cjk_names=?1",
                rusqlite::params![on],
            )
            .unwrap();
        cc_switch_lib::aicli::privacy::layer::bump_revision();
    };
    let (fixture, request) = (&f, &body);
    let send = move || async move {
        let out = fixture
            .client()
            .post(format!("{}/v1/messages", fixture.origin))
            .json(request)
            .send()
            .await
            .unwrap();
        assert_eq!(out.status(), 200);
    };

    send().await; // 默认关
    let upstream = String::from_utf8(f.bodies()[0].clone()).unwrap();
    assert!(
        upstream.contains(&name),
        "off: name forwarded as-is: {upstream}"
    );

    set_switch(true);
    send().await;
    let upstream = String::from_utf8(f.bodies()[1].clone()).unwrap();
    assert!(!upstream.contains(&name), "on: name masked: {upstream}");
    assert!(
        upstream.contains("{{PERSON_"),
        "on: PERSON placeholder: {upstream}"
    );

    set_switch(false);
    send().await;
    let upstream = String::from_utf8(f.bodies()[2].clone()).unwrap();
    assert!(
        upstream.contains(&name),
        "off again: name forwarded: {upstream}"
    );
    f.stop().await;
}
