//! E-06：隐私中间件（`proxy/server.rs` 的 `from_fn_with_state`，位于
//! C3-08 本地鉴权之内）。
//!
//! 请求：读 body → 按 Content-Encoding 解压并移除该头 → `mask_json` →
//! 重建请求（更新 content-length、`Accept-Encoding: identity`）。
//! 响应：SSE 包装流式还原（[`StreamRestorer::shared`]，短锁不跨 .await）；
//! 其他整体 `restore_json`。
//!
//! 引擎经 [`PrivacyGate`] 在运行时构造一次、跨请求共享（同值同占位符）；
//! 设置缓存在内存，`set_privacy` 提交后以全局修订号失效。引擎异常用
//! `catch_unwind` 兜住：隐私优先阻断（`privacy_engine_failed`），可用性
//! 优先放行原文并计数降级。日志只记数量，不记原文与占位符。

use std::panic::AssertUnwindSafe;
#[cfg(feature = "test-hooks")]
use std::sync::atomic::AtomicU8;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use serde_json::json;

use super::placeholder::PlaceholderEngine;
use super::rules::redact::MaskOptions;
use super::stream::StreamRestorer;
use super::walker;
use super::walker::mask_json;
use crate::aicli::secrets::SecretStore;
use crate::proxy::content_encoding;
use crate::proxy::server::ProxyState;
use crate::Database;

/// 与 router 的 DefaultBodyLimit 一致。
const BODY_LIMIT: usize = 200 * 1024 * 1024;

/// 设置修订号：`set_privacy` 等命令提交后 +1；各 [`PrivacyGate`] 据此
/// 失效内存缓存（跨运行时实例也一致，命令与代理不必共享同一运行时）。
static PRIVACY_REVISION: AtomicU64 = AtomicU64::new(0);

/// 命令层在隐私设置落库后调用（aicli_set_privacy / 绑定状态变化）。
pub fn bump_revision() {
    PRIVACY_REVISION.fetch_add(1, Ordering::SeqCst);
}

fn revision() -> u64 {
    PRIVACY_REVISION.load(Ordering::SeqCst)
}

/// 隐私设置快照（`aicli_account` 表三列的内存缓存）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrivacySettings {
    pub enabled: bool,
    /// true = block（隐私优先）；false = pass（可用性优先）。
    pub block: bool,
    pub allow_unscanned_media: bool,
}

impl Default for PrivacySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            block: true,
            allow_unscanned_media: false,
        }
    }
}

/// 故障注入位置（test-hooks；0 = 不注入）。
pub const FAULT_MASK: u8 = 1;
pub const FAULT_RESTORE_JSON: u8 = 2;
pub const FAULT_STREAM: u8 = 3;

/// 引擎失败（构造失败 / panic 已被 catch_unwind 兜住）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineFailure;

/// 隐私网关：引擎构造一次、设置内存缓存、计数器。挂在 [`crate::aicli::runtime::AicliRuntime`]
/// 上跨请求共享；所有锁都是短锁（std），不跨 `.await`。
pub struct PrivacyGate {
    db: Arc<Database>,
    store: Arc<dyn SecretStore>,
    inner: RwLock<GateInner>,
    // 计数器：只记数量与类别，不记内容。
    masked_values: AtomicU64,
    degraded: AtomicU64,
    media_passed: AtomicU64,
    bodies_read: AtomicU64,
    #[cfg(feature = "test-hooks")]
    fault: AtomicU8,
}

struct GateInner {
    engine: Option<Arc<RwLock<PlaceholderEngine>>>,
    /// 引擎构造失败（钥匙串不可用等）：按模式处理，不再每次重试钥匙串。
    engine_failed: bool,
    /// 设置缓存：None = 未读库；(修订号, 快照)。
    cached: Option<(u64, PrivacySettings)>,
}

impl PrivacyGate {
    pub fn new(db: Arc<Database>, store: Arc<dyn SecretStore>) -> Self {
        Self {
            db,
            store,
            inner: RwLock::new(GateInner {
                engine: None,
                engine_failed: false,
                cached: None,
            }),
            masked_values: AtomicU64::new(0),
            degraded: AtomicU64::new(0),
            media_passed: AtomicU64::new(0),
            bodies_read: AtomicU64::new(0),
            #[cfg(feature = "test-hooks")]
            fault: AtomicU8::new(0),
        }
    }

    /// 设置快照：修订号没变走内存缓存；变了重读一行（本地 SQLite 短查询）。
    pub fn settings(&self) -> PrivacySettings {
        let rev = revision();
        {
            let guard = self.inner.read().unwrap_or_else(|p| p.into_inner());
            if let Some((cached_rev, settings)) = guard.cached {
                if cached_rev == rev {
                    return settings;
                }
            }
        }
        self.refresh();
        self.inner
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .cached
            .map(|(_, settings)| settings)
            .unwrap_or_default()
    }

    /// 强制重读设置并缓存。
    pub fn refresh(&self) {
        let row = self.db.conn.lock().ok().and_then(|conn| {
            conn.query_row(
                "SELECT privacy_enabled, privacy_failure_mode, allow_unscanned_media
                     FROM aicli_account WHERE id=1",
                [],
                |row| {
                    Ok((
                        row.get::<_, bool>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, bool>(2)?,
                    ))
                },
            )
            .ok()
        });
        let settings = match row {
            Some((enabled, mode, allow_media)) => PrivacySettings {
                enabled,
                block: mode != "pass",
                allow_unscanned_media: allow_media,
            },
            None => PrivacySettings::default(),
        };
        let mut guard = self.inner.write().unwrap_or_else(|p| p.into_inner());
        guard.cached = Some((revision(), settings));
    }

    /// 引擎（懒构造一次）：占位符密钥只在首次使用时读一次钥匙串。
    pub fn engine(&self) -> Option<Arc<RwLock<PlaceholderEngine>>> {
        {
            let guard = self.inner.read().unwrap_or_else(|p| p.into_inner());
            if let Some(engine) = &guard.engine {
                return Some(engine.clone());
            }
            if guard.engine_failed {
                return None;
            }
        }
        let mut guard = self.inner.write().unwrap_or_else(|p| p.into_inner());
        if let Some(engine) = &guard.engine {
            return Some(engine.clone());
        }
        match PlaceholderEngine::from_store(self.store.as_ref()) {
            Ok(engine) => {
                let engine = Arc::new(RwLock::new(engine));
                guard.engine = Some(engine.clone());
                Some(engine)
            }
            Err(error) => {
                guard.engine_failed = true;
                log::error!("隐私引擎初始化失败：{error}");
                None
            }
        }
    }

    /// 计数器访问（首页/测试用；只有数量）。
    pub fn stats(&self) -> PrivacyStats {
        PrivacyStats {
            masked_values: self.masked_values.load(Ordering::Relaxed),
            degraded: self.degraded.load(Ordering::Relaxed),
            media_passed: self.media_passed.load(Ordering::Relaxed),
            bodies_read: self.bodies_read.load(Ordering::Relaxed),
        }
    }

    fn note_masked(&self, count: u64) {
        self.masked_values.fetch_add(count, Ordering::Relaxed);
    }

    fn note_degraded(&self) {
        self.degraded.fetch_add(1, Ordering::Relaxed);
        log::error!("隐私引擎降级：本次请求未脱敏/未还原（可用性优先）");
    }

    fn note_media_passed(&self) {
        self.media_passed.fetch_add(1, Ordering::Relaxed);
    }

    fn note_body_read(&self) {
        self.bodies_read.fetch_add(1, Ordering::Relaxed);
    }

    /// 遮盖请求体。`Ok((text, count))`；`Err(())` = 引擎异常（含注入），
    /// 调用方按模式处理。
    pub fn mask_request(&self, text: &str) -> Result<(String, usize), EngineFailure> {
        let engine = self.engine().ok_or(EngineFailure)?;
        let options = MaskOptions {
            secrets: true,
            personal: true,
            ..Default::default()
        };
        #[cfg(feature = "test-hooks")]
        let fault = self.injected_failure();
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            #[cfg(feature = "test-hooks")]
            if fault == FAULT_MASK {
                panic!("privacy fault injection: mask");
            }
            let mut guard = engine.write().unwrap_or_else(|p| p.into_inner());
            mask_json(text, &options, &mut guard)
        }));
        match result {
            Ok((out, count)) => {
                self.note_masked(count as u64);
                Ok((out, count))
            }
            Err(payload) => {
                // mask 只在完整算出占位符后才写值表；中毒后继续用是安全的。
                log::error!("隐私引擎遮盖 panic：{}", panic_message(&payload));
                Err(EngineFailure)
            }
        }
    }

    /// 整体还原响应体。`Err(())` = 引擎异常。
    pub fn restore_response(&self, text: &str) -> Result<String, EngineFailure> {
        let engine = self.engine().ok_or(EngineFailure)?;
        #[cfg(feature = "test-hooks")]
        let fault = self.injected_failure();
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            #[cfg(feature = "test-hooks")]
            if fault == FAULT_RESTORE_JSON {
                panic!("privacy fault injection: restore_json");
            }
            let guard = engine.read().unwrap_or_else(|p| p.into_inner());
            walker::restore_json(text, &guard)
        }));
        result.map_err(|payload| {
            log::error!("隐私引擎还原 panic：{}", panic_message(&payload));
            EngineFailure
        })
    }

    /// 流式还原器（共享引擎版；注入点在 RestoreStream::restore_chunk）。
    pub fn stream_restorer(&self) -> Option<StreamRestorer<'static>> {
        let engine = self.engine()?;
        Some(StreamRestorer::shared(engine))
    }

    /// test-hooks：注入引擎 panic（0 清除）。
    #[cfg(feature = "test-hooks")]
    pub fn inject_failure(&self, position: u8) {
        self.fault.store(position, Ordering::SeqCst);
    }

    #[cfg(feature = "test-hooks")]
    pub fn injected_failure(&self) -> u8 {
        self.fault.load(Ordering::SeqCst)
    }
}

/// 计数快照。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrivacyStats {
    pub masked_values: u64,
    pub degraded: u64,
    pub media_passed: u64,
    pub bodies_read: u64,
}

fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown panic")
}

// ---- 中间件 ----

/// E-06 隐私中间件：`.layer()` 写在 `local_auth` 之前（axum 后加的 layer
/// 在外层，鉴权先执行；隐私层在读 body 之前必须已通过鉴权）。
pub async fn privacy_middleware(
    State(state): State<ProxyState>,
    request: Request,
    next: Next,
) -> Response {
    let gate = Arc::clone(state.aicli.privacy());
    let settings = gate.settings();
    if !settings.enabled {
        return next.run(request).await;
    }
    if request.method() != axum::http::Method::POST {
        return next.run(request).await;
    }
    let path = request.uri().path().to_owned();
    if is_media_path(&path) {
        // 图片生成/编辑接口：内容是 base64 媒体，规则无法遮盖。隐私优先
        // 下直接说明不支持；可用性优先放行。
        if settings.block {
            return privacy_error(
                StatusCode::BAD_REQUEST,
                "privacy_media_unsupported",
                "隐私开启（隐私优先）时图片生成与编辑接口不支持：请求体是未扫描的 base64 媒体。可在设置中切换为可用性优先，或关闭隐私后使用。",
            );
        }
        gate.note_media_passed();
        return next.run(request).await;
    }
    if !is_ai_path(&path) {
        return next.run(request).await;
    }

    // ---- 请求方向 ----
    gate.note_body_read();
    let (mut parts, body) = request.into_parts();
    let bytes = match axum::body::to_bytes(body, BODY_LIMIT).await {
        Ok(bytes) => bytes,
        Err(error) => {
            log::error!("隐私层读请求体失败：{error}");
            return privacy_error(
                StatusCode::BAD_REQUEST,
                "privacy_engine_failed",
                "隐私层无法读取请求体。",
            );
        }
    };
    // 解压（并移除 content-encoding）；无法解压时按模式处理
    let plain: Vec<u8> = match content_encoding::get_content_encoding(&parts.headers) {
        Some(encoding) => match content_encoding::decompress_body(&encoding, &bytes) {
            Ok(Some(plain)) => {
                parts.headers.remove(header::CONTENT_ENCODING);
                plain
            }
            _ => {
                if settings.block {
                    return privacy_error(
                        StatusCode::BAD_REQUEST,
                        "privacy_engine_failed",
                        "请求体使用了不支持的 Content-Encoding，隐私层无法遮盖。",
                    );
                }
                gate.note_degraded();
                bytes.to_vec()
            }
        },
        None => bytes.to_vec(),
    };
    let Ok(text) = String::from_utf8(plain) else {
        if settings.block {
            return privacy_error(
                StatusCode::BAD_REQUEST,
                "privacy_engine_failed",
                "请求体不是 UTF-8 文本，隐私层无法遮盖。",
            );
        }
        gate.note_degraded();
        return finish_request(next, parts, bytes, &gate, settings).await;
    };
    // 含 base64 图片/文件：隐私优先且未放行 → 阻断；否则继续遮盖
    //（mask_json 对 data: 值原样保留，只遮文本）并计数
    if contains_unscannable_media(&text) && !settings.allow_unscanned_media && settings.block {
        return privacy_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "privacy_media_blocked",
            "请求含 base64 图片或文件，隐私层无法扫描。可在隐私设置中允许未扫描媒体（allow_unscanned_media）后重试。",
        );
    }
    if contains_unscannable_media(&text) {
        gate.note_media_passed();
    }
    let masked = gate.mask_request(&text);
    let payload: Bytes = match masked {
        Ok((masked, _count)) => Bytes::from(masked),
        Err(EngineFailure) => {
            if settings.block {
                return privacy_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "privacy_engine_failed",
                    "隐私引擎遮盖失败，请求已按隐私优先策略阻断。",
                );
            }
            gate.note_degraded();
            bytes
        }
    };
    finish_request(next, parts, payload, &gate, settings).await
}

/// 重建请求（content-length、Accept-Encoding: identity）并继续，随后处理
/// 响应方向。
async fn finish_request(
    next: Next,
    mut parts: axum::http::request::Parts,
    payload: Bytes,
    gate: &Arc<PrivacyGate>,
    settings: PrivacySettings,
) -> Response {
    let headers = &mut parts.headers;
    headers.remove(header::CONTENT_LENGTH);
    if let Ok(value) = HeaderValue::from_str(&payload.len().to_string()) {
        headers.insert(header::CONTENT_LENGTH, value);
    }
    headers.insert(
        header::ACCEPT_ENCODING,
        HeaderValue::from_static("identity"),
    );
    let request = Request::from_parts(parts, Body::from(payload));
    let response = next.run(request).await;
    response_with_gate(gate, settings, response).await
}

/// 响应方向实现（带网关）。
async fn response_with_gate(
    gate: &Arc<PrivacyGate>,
    settings: PrivacySettings,
    response: Response,
) -> Response {
    let is_sse = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("text/event-stream"));
    let compressed = content_encoding::get_content_encoding(response.headers()).is_some();
    if is_sse {
        if compressed {
            // fail-closed：无法还原压缩的流式内容。隐私优先返回错误；
            // 可用性优先原样透传（客户端按 content-encoding 自行解压）。
            if settings.block {
                return privacy_error(
                    StatusCode::BAD_GATEWAY,
                    "privacy_compressed_stream",
                    "上游以压缩返回流式内容，隐私层无法流式还原，已按隐私优先策略阻断。",
                );
            }
            gate.note_degraded();
            return response;
        }
        let Some(restorer) = gate.stream_restorer() else {
            if settings.block {
                return privacy_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "privacy_engine_failed",
                    "隐私引擎不可用，响应已按隐私优先策略阻断。",
                );
            }
            gate.note_degraded();
            return response;
        };
        let (mut parts, body) = response.into_parts();
        let stream = RestoreStream {
            upstream: Box::pin(body.into_data_stream()),
            restorer,
            gate: Arc::clone(gate),
            block: settings.block,
            passthrough: false,
            finished: false,
            #[cfg(feature = "test-hooks")]
            fault: gate.injected_failure(),
        };
        parts.headers.remove(header::CONTENT_LENGTH);
        let mut response = Response::from_parts(parts, Body::from_stream(stream));
        response.headers_mut().remove(header::CONTENT_LENGTH);
        return response;
    }
    // 整体还原（非流式与错误响应同样处理）
    let (mut parts, body) = response.into_parts();
    let bytes = match axum::body::to_bytes(body, BODY_LIMIT).await {
        Ok(bytes) => bytes,
        Err(_) => {
            // 读不完整的响应原样返回
            let mut response = Response::from_parts(parts, Body::from(Bytes::new()));
            *response.status_mut() = StatusCode::BAD_GATEWAY;
            return response;
        }
    };
    let plain: Vec<u8> = match content_encoding::get_content_encoding(&parts.headers) {
        Some(encoding) => match content_encoding::decompress_body(&encoding, &bytes) {
            Ok(Some(plain)) => {
                parts.headers.remove(header::CONTENT_ENCODING);
                plain
            }
            _ => {
                // 无法解压：非流式内容按模式决定
                let _ = gate;
                return Response::from_parts(parts, Body::from(bytes));
            }
        },
        None => bytes.to_vec(),
    };
    let Ok(text) = String::from_utf8(plain) else {
        // 非 UTF-8 响应（二进制）不含占位符，原样返回
        return Response::from_parts(parts, Body::from(bytes));
    };
    match gate.restore_response(&text) {
        Ok(restored) => {
            let payload = Bytes::from(restored);
            parts.headers.remove(header::CONTENT_LENGTH);
            if let Ok(value) = HeaderValue::from_str(&payload.len().to_string()) {
                parts.headers.insert(header::CONTENT_LENGTH, value);
            }
            Response::from_parts(parts, Body::from(payload))
        }
        Err(EngineFailure) => {
            if settings.block {
                privacy_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "privacy_engine_failed",
                    "隐私引擎还原失败，响应已按隐私优先策略阻断。",
                )
            } else {
                gate.note_degraded();
                Response::from_parts(parts, Body::from(bytes))
            }
        }
    }
}

/// SSE 流包装：每块短锁还原；panic/注入按模式处理。
struct RestoreStream {
    upstream: std::pin::Pin<Box<dyn futures::Stream<Item = Result<Bytes, axum::Error>> + Send>>,
    restorer: StreamRestorer<'static>,
    gate: Arc<PrivacyGate>,
    block: bool,
    /// 可用性优先下引擎失败后：剩余字节原样透传（其中只有占位符）。
    passthrough: bool,
    finished: bool,
    #[cfg(feature = "test-hooks")]
    fault: u8,
}

impl RestoreStream {
    fn restore_chunk(&mut self, chunk: &[u8]) -> String {
        #[cfg(feature = "test-hooks")]
        if self.fault == FAULT_STREAM {
            panic!("privacy fault injection: stream");
        }
        self.restorer.push(chunk)
    }
}

impl futures::Stream for RestoreStream {
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        loop {
            if self.finished {
                return std::task::Poll::Ready(None);
            }
            if self.passthrough {
                return match self.upstream.as_mut().poll_next(cx) {
                    std::task::Poll::Ready(Some(Ok(bytes))) => {
                        std::task::Poll::Ready(Some(Ok(bytes)))
                    }
                    std::task::Poll::Ready(Some(Err(error))) => {
                        std::task::Poll::Ready(Some(Err(std::io::Error::other(error))))
                    }
                    std::task::Poll::Ready(None) => std::task::Poll::Ready(None),
                    std::task::Poll::Pending => std::task::Poll::Pending,
                };
            }
            let this = &mut *self;
            match this.upstream.as_mut().poll_next(cx) {
                std::task::Poll::Ready(Some(Ok(chunk))) => {
                    match std::panic::catch_unwind(AssertUnwindSafe(|| this.restore_chunk(&chunk)))
                    {
                        Ok(out) => {
                            if out.is_empty() {
                                continue; // 全部扣留/注释未产出：继续拉
                            }
                            return std::task::Poll::Ready(Some(Ok(Bytes::from(out))));
                        }
                        Err(payload) => {
                            log::error!("隐私引擎流式还原 panic：{}", panic_message(&payload));
                            this.gate.note_degraded();
                            if this.block {
                                // 头已发出：立即结束流，不再透传上游字节
                                this.finished = true;
                                return std::task::Poll::Ready(None);
                            }
                            this.passthrough = true;
                            return std::task::Poll::Ready(Some(Ok(chunk)));
                        }
                    }
                }
                std::task::Poll::Ready(Some(Err(error))) => {
                    // 放出扣留内容后随错误结束
                    let _ = self.restorer.finish();
                    self.finished = true;
                    return std::task::Poll::Ready(Some(Err(std::io::Error::other(error))));
                }
                std::task::Poll::Ready(None) => {
                    let out = self.restorer.finish();
                    self.finished = true;
                    if out.is_empty() {
                        return std::task::Poll::Ready(None);
                    }
                    return std::task::Poll::Ready(Some(Ok(Bytes::from(out))));
                }
                std::task::Poll::Pending => return std::task::Poll::Pending,
            }
        }
    }
}

// ---- 路径与媒体识别 ----

/// AI 路径的 POST（清单：/v1/messages、/chat/completions、/responses、
/// Gemini generateContent 系列及前缀变体、/app/*；alpha/search 同为
/// 文本入参，一并遮盖）。
pub(crate) fn is_ai_path(path: &str) -> bool {
    path.ends_with("/messages")
        || path.ends_with("/chat/completions")
        || path.contains("/responses")
        || path.contains(":generateContent")
        || path.contains(":streamGenerateContent")
        || path.contains(":countTokens")
        || path == "/app"
        || path.starts_with("/app/")
        || path.ends_with("/alpha/search")
}

/// 图片生成/编辑接口（内容是 base64 媒体）。
pub(crate) fn is_media_path(path: &str) -> bool {
    path.ends_with("/images/generations") || path.ends_with("/images/edits")
}

/// 请求体是否含未扫描媒体：data: URL 或 base64 文件字段（walk 只看形态，
/// 不改字节）。
pub(crate) fn contains_unscannable_media(body: &str) -> bool {
    let mut found = false;
    let _ = walker::walk(body, &mut |_path, key, s| {
        if s.starts_with("data:") || matches!(key, "file_data" | "binary_data") {
            found = true;
        }
        s.to_string()
    });
    found
}

fn privacy_error(status: StatusCode, code: &str, message: &str) -> Response {
    let body = json!({
        "error": {
            "type": "privacy",
            "code": code,
            "message": message,
        }
    });
    (status, axum::Json(body)).into_response()
}

#[cfg(test)]
mod tests;
