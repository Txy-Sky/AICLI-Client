//! E-GATE 第 2 项：真实 Claude Code 隐私轮的常驻门禁服务（手动运行，不在
//! aicli-check.sh 里跑）。
//!
//! 形态（交接 E-GATE.md）：捕获上游 + 真实 ProxyServer（隐私开启），Claude Code
//! 的 ANTHROPIC_BASE_URL 指向本代理。本进程：
//!   1. 在 EGATE_UPSTREAM_PORT（默认 18991）起脚本化捕获上游：每个请求
//!      （path/body）追加写入 $EGATE_STATE_DIR/capture.jsonl；按请求内容回
//!      thinking + 工具调用 / 回执 / 慢流（供 Esc 中断）三种 SSE；
//!   2. 在 EGATE_PROXY_PORT（默认 18992）起真实 ProxyServer（合成 aicli 账号
//!      privacy_enabled=1，同 aicli_privacy_e2e 的夹具）；
//!   3. 把代理日志追加写入 $EGATE_STATE_DIR/proxy.log（供「日志查不到假密钥、
//!      无 error」断言）；
//!   4. 写 $EGATE_STATE_DIR/info.json（端口 / 本地令牌 / 模型别名），然后驻留
//!      直到 $EGATE_STATE_DIR/stop 出现。
//!
//! 运行：cargo test --manifest-path src-tauri/Cargo.toml --features test-hooks
//!       --test aicli_egate_gate -- --ignored --nocapture
//! 断言与打码出证由 scripts/aicli-egate-privacy.sh 完成。

#![cfg(feature = "test-hooks")]

#[path = "support.rs"]
#[allow(dead_code)]
mod support;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::response::IntoResponse as _;
use cc_switch_lib::aicli::profiles::{ModelBinding, ProfileStore};
use cc_switch_lib::aicli::routing_test_support::{
    self as routing_support, ProxyConfig, ProxyServer,
};
use cc_switch_lib::aicli::runtime::{AicliRuntime, RuntimeRemote, RuntimeValues};
use cc_switch_lib::aicli::secrets::{SecretError, SecretStore};
use cc_switch_lib::aicli::{
    newapi, seed,
    tokens::{groups_hash, TokenRemote},
};
use cc_switch_lib::{AppType, Database};
use futures::future::BoxFuture;
use serde_json::{json, Value};

const UPSTREAM: &str = "upstream-fixture";
/// 用真实模型名做公开别名：Claude Code 能从自己的目录里识别它（避免未知模型
/// 的上下文窗口强制等待），代理按别名映射到 upstream-fixture。
const PUBLIC: &str = "claude-sonnet-4-5-20250929";

// ---- 状态目录（由驱动脚本提供） ----

fn state_dir() -> PathBuf {
    PathBuf::from(std::env::var("EGATE_STATE_DIR").expect("EGATE_STATE_DIR is required"))
}

fn append_line(name: &str, line: &str) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(state_dir().join(name))
        .expect("open state file");
    writeln!(file, "{line}").expect("write state file");
}

// ---- 代理日志落盘（断言无敏感值 / 无 error 用） ----

struct FileLog;
impl log::Log for FileLog {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, record: &log::Record) {
        let line = format!(
            "{} {:<5} {}",
            chrono::Utc::now().format("%H:%M:%S%.3f"),
            record.level(),
            record.args()
        );
        append_line("proxy.log", &line);
    }
    fn flush(&self) {}
}

// ---- 离线 remote / 内存密钥库（同 aicli_privacy_e2e） ----

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
        _: &'a newapi::NewApiClient,
    ) -> BoxFuture<'a, Result<RuntimeValues, newapi::NewApiError>> {
        Box::pin(async { Err(newapi::NewApiError::Network) })
    }
    fn tokens(&self, _: newapi::NewApiClient) -> Arc<dyn TokenRemote> {
        Arc::new(Self)
    }
}
impl TokenRemote for Offline {
    fn list(&self) -> BoxFuture<'_, Result<Vec<Value>, newapi::NewApiError>> {
        Box::pin(async { Err(newapi::NewApiError::Network) })
    }
    fn create<'a>(&'a self, _: &'a Value) -> BoxFuture<'a, Result<(), newapi::NewApiError>> {
        Box::pin(async { Err(newapi::NewApiError::Network) })
    }
    fn keys<'a>(
        &'a self,
        _: &'a [i64],
    ) -> BoxFuture<'a, Result<BTreeMap<i64, String>, newapi::NewApiError>> {
        Box::pin(async { Err(newapi::NewApiError::Network) })
    }
}

// ---- 脚本化捕获上游 ----

/// 请求里的引擎占位符（{{KIND_xxxxxxxx}}，KIND 大写/下划线、8 位小写后缀）。
/// Claude Code 的系统提示里自带 {{模板}} 文案，不是占位符，不算进来。
/// 响应路径会被代理还原成原值——这正是「Claude Code 见原值」的断言来源。
fn placeholders(body: &str) -> Vec<String> {
    static PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern = PATTERN
        .get_or_init(|| regex::Regex::new(r"\{\{[A-Z][A-Z0-9_]{0,23}_[a-z0-9]{8}\}\}").unwrap());
    pattern
        .find_iter(body)
        .map(|m| m.as_str().to_owned())
        .collect()
}

/// 最后一条 user 文本（工具轮判断 + 场景标记用）。
fn last_user_text(body: &Value) -> String {
    let messages = body["messages"].as_array();
    let mut text = String::new();
    for message in messages.into_iter().flatten() {
        if message["role"] == "user" {
            match &message["content"] {
                Value::String(s) => text = s.clone(),
                Value::Array(blocks) => {
                    let joined: Vec<&str> =
                        blocks.iter().filter_map(|b| b["text"].as_str()).collect();
                    if !joined.is_empty() {
                        text = joined.join(" ");
                    }
                }
                _ => {}
            }
        }
    }
    text
}

fn has_tool_result(body: &Value) -> bool {
    body["messages"].as_array().is_some_and(|messages| {
        messages.iter().any(|message| {
            message["content"]
                .as_array()
                .is_some_and(|blocks| blocks.iter().any(|b| b["type"] == "tool_result"))
        })
    })
}

fn sse_event(event: &str, data: &Value) -> String {
    format!("event: {event}\ndata: {}\n\n", data)
}

/// 按场景构造完整 Anthropic SSE 文本。
/// - 工具轮（EGATE-TOOL 且尚无 tool_result）：thinking + 文本 + Bash 工具调用；
///   Claude Code 执行后会带 tool_result 回来，走回执分支。
/// - 回执轮（有 tool_result 或无标记）：thinking + 回执文本（列出收到的占位符）。
/// - 中断轮（EGATE-INTERRUPT）：长文本慢速下发，供中途 Esc/杀进程。
fn build_sse(
    body_text: &str,
    tool_round: bool,
    placeholders: &[String],
) -> (String, Option<Duration>) {
    let mut frames = String::new();
    frames.push_str(&sse_event(
        "message_start",
        &json!({"type":"message_start","message":{"id":"msg_egate_fixture","type":"message",
            "role":"assistant","model":UPSTREAM,"content":[],"stop_reason":null,
            "usage":{"input_tokens":64,"output_tokens":1}}}),
    ));
    // thinking 块
    frames.push_str(&sse_event(
        "content_block_start",
        &json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}),
    ));
    // thinking 里带上收到的占位符：响应还原须覆盖 thinking_delta
    frames.push_str(&sse_event(
        "content_block_delta",
        &json!({"type":"content_block_delta","index":0,
            "delta":{"type":"thinking_delta","thinking":format!(
                "E-GATE fixture thinking: checking the request payload. seen {}",
                placeholders.join(" "))}}),
    ));
    if std::env::var("EGATE_SIGNATURE").is_ok() {
        frames.push_str(&sse_event(
            "content_block_delta",
            &json!({"type":"content_block_delta","index":0,
                "delta":{"type":"signature_delta","signature":"EgICgIBgBQoECkCAgAE"}}),
        ));
    }
    frames.push_str(&sse_event(
        "content_block_stop",
        &json!({"type":"content_block_stop","index":0}),
    ));

    let slow = body_text.contains("EGATE-INTERRUPT");
    let receipt = format!(
        "捕获上游回执：共收到 {} 个占位符：{}",
        placeholders.len(),
        placeholders.join(" ")
    );

    // 文本块
    frames.push_str(&sse_event(
        "content_block_start",
        &json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}),
    ));
    if slow {
        for index in 0..24 {
            frames.push_str(&sse_event(
                "content_block_delta",
                &json!({"type":"content_block_delta","index":1,
                    "delta":{"type":"text_delta","text":format!("slow-chunk-{index:02} ")}}),
            ));
        }
    } else {
        frames.push_str(&sse_event(
            "content_block_delta",
            &json!({"type":"content_block_delta","index":1,
                "delta":{"type":"text_delta","text":&receipt}}),
        ));
    }
    frames.push_str(&sse_event(
        "content_block_stop",
        &json!({"type":"content_block_stop","index":1}),
    ));

    let stop_reason = if tool_round { "tool_use" } else { "end_turn" };
    if tool_round {
        frames.push_str(&sse_event(
            "content_block_start",
            &json!({"type":"content_block_start","index":2,
                "content_block":{"type":"tool_use","id":"toolu_egate_fixture","name":"Bash","input":{}}}),
        ));
        // 工具命令带收到的 API_KEY 占位符，并从占位符中间切成两个
        // input_json_delta：代理须跨片段拼齐还原，Claude Code 执行的才是原值
        let key = placeholders
            .iter()
            .find(|p| p.starts_with("{{API_KEY_"))
            .cloned()
            .unwrap_or_default();
        let command = format!("{{\"command\":\"echo egate-tool-ok {key}\"}}");
        let cut = command
            .find("{{API_KEY_")
            .map_or(command.len() / 2, |i| i + 6);
        for part in [&command[..cut], &command[cut..]] {
            frames.push_str(&sse_event(
                "content_block_delta",
                &json!({"type":"content_block_delta","index":2,
                    "delta":{"type":"input_json_delta","partial_json":part}}),
            ));
        }
        frames.push_str(&sse_event(
            "content_block_stop",
            &json!({"type":"content_block_stop","index":2}),
        ));
    }
    frames.push_str(&sse_event(
        "message_delta",
        &json!({"type":"message_delta","delta":{"stop_reason":stop_reason},"usage":{"output_tokens":128}}),
    ));
    frames.push_str(&sse_event("message_stop", &json!({"type":"message_stop"})));
    let pace = if slow {
        Some(Duration::from_millis(900))
    } else {
        None
    };
    (frames, pace)
}

/// 非流式回复（count_tokens / 兜底）：与 SSE 同等内容。
fn json_reply(path: &str, placeholders: &[String]) -> axum::response::Response {
    if path.contains("count_tokens") {
        return axum::Json(json!({"input_tokens": 128})).into_response();
    }
    let receipt = format!(
        "捕获上游回执：共收到 {} 个占位符：{}",
        placeholders.len(),
        placeholders.join(" ")
    );
    axum::Json(json!({"id":"msg_egate_fixture","type":"message","role":"assistant","model":UPSTREAM,
        "content":[
            {"type":"thinking","thinking":"E-GATE fixture thinking.","signature":"EgICgIBgBQoECkCAgAE"},
            {"type":"text","text":receipt}],
        "stop_reason":"end_turn","usage":{"input_tokens":64,"output_tokens":64}}))
    .into_response()
}

async fn upstream(request: axum::extract::Request) -> axum::response::Response {
    let path = request.uri().path().to_owned();
    let bytes = axum::body::to_bytes(request.into_body(), 200 * 1024 * 1024)
        .await
        .unwrap()
        .to_vec();
    let parsed: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let record = json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "path": path,
        "stream": parsed.get("stream").and_then(Value::as_bool).unwrap_or(false),
        "model": parsed.get("model").and_then(Value::as_str),
        "body": String::from_utf8_lossy(&bytes),
    });
    append_line("capture.jsonl", &record.to_string());

    if path.contains("count_tokens") || parsed.get("stream") != Some(&Value::Bool(true)) {
        let found = placeholders(&String::from_utf8_lossy(&bytes));
        return json_reply(&path, &found);
    }

    let body_text = last_user_text(&parsed);
    let found = placeholders(&String::from_utf8_lossy(&bytes));
    let tool_round = body_text.contains("EGATE-TOOL") && !has_tool_result(&parsed);
    let (frames, pace) = build_sse(&body_text, tool_round, &found);
    let events: Vec<bytes::Bytes> = frames
        .split("\n\n")
        .filter(|frame| !frame.is_empty())
        .map(|frame| bytes::Bytes::from(format!("{frame}\n\n")))
        .collect();
    let stream = async_stream::stream! {
        for event in events {
            if let Some(pace) = pace {
                tokio::time::sleep(pace).await;
            }
            yield Ok::<_, std::io::Error>(event);
        }
    };
    let mut response = axum::response::Response::builder()
        .header("content-type", "text/event-stream")
        .body(axum::body::Body::from_stream(stream))
        .unwrap();
    response
        .headers_mut()
        .insert("x-oneapi-request-id", "egate-upstream".parse().unwrap());
    response
}

// ---- 门禁主体 ----

#[tokio::test]
#[ignore = "E-GATE 手动隐私轮：由 scripts/aicli-egate-privacy.sh 驱动（EGATE_STATE_DIR 必填）"]
async fn egate_privacy_gate_serve() {
    support::ensure_test_home();
    // 与生产 app setup 相同：安装 rustls CryptoProvider（代理的 https
    // 客户端路径需要；测试进程没人装会 panic）
    let _ = rustls::crypto::ring::default_provider().install_default();
    let _ = log::set_boxed_logger(Box::new(FileLog)).map(|_| ());
    log::set_max_level(log::LevelFilter::Info);

    let dir = state_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let upstream_port: u16 = std::env::var("EGATE_UPSTREAM_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(18991);
    let proxy_port: u16 = std::env::var("EGATE_PROXY_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(18992);
    let mock_origin = format!("http://127.0.0.1:{upstream_port}");

    // 捕获上游
    let upstream_listener = tokio::net::TcpListener::bind(("127.0.0.1", upstream_port))
        .await
        .expect("bind upstream port");
    let upstream_app = axum::Router::new().fallback(upstream);
    let upstream_task =
        tokio::spawn(async move { axum::serve(upstream_listener, upstream_app).await.unwrap() });

    // 合成 aicli 账号 + 隐私开启（同 e2e 夹具，block 模式：遮不住就失败，
    // 不许裸值出门）
    let db = Arc::new(Database::memory().unwrap());
    seed::init(&db).unwrap();
    let store = Arc::new(Memory::default());
    store.set("access-token", "egate-synthetic-access").unwrap();
    store.set("token:17", "egate-synthetic-model-key").unwrap();
    {
        let conn = routing_support::connection(&db);
        conn.execute(
            "INSERT INTO aicli_account(id,user_id,username,user_group,privacy_enabled,privacy_failure_mode)
             VALUES(1,17,'egate-fixture','fixture',1,'block')",
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

    // 配置集：alias friendly-fixture → upstream-fixture，Claude 客户端经
    // aicli-claude- 前缀别名访问
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
    let origin_for_closure = mock_origin.clone();
    runtime.set_local_test_provider_source(move |app, line, wire| {
        let mut provider = seed::clone_for(app, line, wire).unwrap();
        match app {
            AppType::Claude | AppType::ClaudeDesktop => {
                provider.settings_config["env"]["ANTHROPIC_BASE_URL"] = json!(origin_for_closure)
            }
            AppType::Gemini => {
                provider.settings_config["env"]["GOOGLE_GEMINI_BASE_URL"] =
                    json!(origin_for_closure)
            }
            _ => provider.settings_config["base_url"] = json!(origin_for_closure),
        }
        Some(provider)
    });

    // 真实代理（隐私中间件在 local_auth 之内，生产同款挂载）
    let proxy = ProxyServer::new(
        ProxyConfig {
            listen_port: proxy_port,
            ..Default::default()
        },
        db.clone(),
        None,
    )
    .with_test_runtime(runtime);
    if std::env::var("EGATE_PRIVACY_OFF").is_ok() {
        routing_support::connection(&db)
            .execute("UPDATE aicli_account SET privacy_enabled=0", [])
            .unwrap();
    }
    let info = proxy.start().await.expect("start proxy");
    let token = cc_switch_lib::aicli::local_auth::token_for(&db, "claude").unwrap();
    let info = json!({
        "proxy_port": info.port,
        "upstream_port": upstream_port,
        "token": token,
        "model_alias": PUBLIC,
        "privacy": "enabled/block",
    });
    std::fs::write(dir.join("info.json"), info.to_string()).unwrap();
    eprintln!("E-GATE gate ready: {info}");

    // 驻留直到 stop 文件出现
    let stop = dir.join("stop");
    while !stop.exists() {
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    proxy.stop().await.unwrap();
    upstream_task.abort();
    eprintln!("E-GATE gate stopped");
}
