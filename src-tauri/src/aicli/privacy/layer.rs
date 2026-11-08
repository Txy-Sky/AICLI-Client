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
    /// E-08 中文姓名检测（aicli_account.privacy_detect_cjk_names，默认关）。
    pub detect_cjk_names: bool,
}

impl Default for PrivacySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            block: true,
            allow_unscanned_media: false,
            detect_cjk_names: false,
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
    /// block 模式下的引擎失败（已阻断，不是降级）。
    engine_blocked: AtomicU64,
    media_passed: AtomicU64,
    bodies_read: AtomicU64,
    #[cfg(feature = "test-hooks")]
    fault: AtomicU8,
    /// FAULT_STREAM 在第 N 块触发（默认 1）。
    #[cfg(feature = "test-hooks")]
    fault_stream_at: std::sync::atomic::AtomicU32,
}

struct GateInner {
    engine: Option<Arc<RwLock<PlaceholderEngine>>>,
    /// 引擎构造失败（钥匙串不可用等）：按模式处理，不再每次重试钥匙串。
    engine_failed: bool,
    /// 设置缓存：None = 未读库；(修订号, 快照)。
    cached: Option<(u64, PrivacySettings)>,
    /// 上次查库出错的时间（最短重试间隔内不再打库，期间 fail-closed）。
    last_error_at: Option<std::time::Instant>,
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
                last_error_at: None,
            }),
            masked_values: AtomicU64::new(0),
            degraded: AtomicU64::new(0),
            engine_blocked: AtomicU64::new(0),
            media_passed: AtomicU64::new(0),
            bodies_read: AtomicU64::new(0),
            #[cfg(feature = "test-hooks")]
            fault: AtomicU8::new(0),
            #[cfg(feature = "test-hooks")]
            fault_stream_at: std::sync::atomic::AtomicU32::new(0),
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
        self.refresh()
    }

    /// 重读设置并缓存，返回本次应使用的设置。修订号在查库**前**读取
    ///（bump 与读库的竞态下宁可下次多读一次，不可把旧设置当成新修订）。
    ///
    /// - 查不到行（未绑定）= 关闭，写入缓存；
    /// - **查库出错：本次按 fail-closed（开启 + block）处理，不更新缓存
    ///   修订号**——否则「缓存为关闭 → 开启隐私 → 一次出错」会把关闭固化
    ///   到新修订号下；下次请求自动重试。持续出错时按 1s 最短重试间隔
    ///   限频（期间仍 fail-closed），不逐请求打库。
    pub fn refresh(&self) -> PrivacySettings {
        let rev = revision();
        let fail_closed = PrivacySettings {
            enabled: true,
            block: true,
            allow_unscanned_media: false,
            detect_cjk_names: false,
        };
        // 限频：上次出错不足 1s，直接 fail-closed 不打库
        {
            let guard = self.inner.read().unwrap_or_else(|p| p.into_inner());
            if let Some(last) = guard.last_error_at {
                if last.elapsed() < std::time::Duration::from_secs(1) {
                    return fail_closed;
                }
            }
        }
        let row: Result<Option<(bool, String, bool, bool)>, ()> = (|| {
            use rusqlite::OptionalExtension;
            let conn = self.db.conn.lock().map_err(|_| ())?;
            conn.query_row(
                "SELECT privacy_enabled, privacy_failure_mode, allow_unscanned_media,
                        privacy_detect_cjk_names
                     FROM aicli_account WHERE id=1",
                [],
                |row| {
                    Ok((
                        row.get::<_, bool>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, bool>(2)?,
                        row.get::<_, bool>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| ())
        })();
        let mut guard = self.inner.write().unwrap_or_else(|p| p.into_inner());
        match row {
            Ok(Some((enabled, mode, allow_media, detect_cjk_names))) => {
                let settings = PrivacySettings {
                    enabled,
                    block: mode != "pass",
                    allow_unscanned_media: allow_media,
                    detect_cjk_names,
                };
                guard.cached = Some((rev, settings));
                guard.last_error_at = None;
                settings
            }
            // 查不到行（未绑定）：视为关闭
            Ok(None) => {
                guard.cached = Some((rev, PrivacySettings::default()));
                guard.last_error_at = None;
                PrivacySettings::default()
            }
            // 查库出错：fail-closed，不更新缓存（下次重试），记录限频时间
            Err(()) => {
                guard.last_error_at = Some(std::time::Instant::now());
                fail_closed
            }
        }
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
            engine_blocked: self.engine_blocked.load(Ordering::Relaxed),
            media_passed: self.media_passed.load(Ordering::Relaxed),
            bodies_read: self.bodies_read.load(Ordering::Relaxed),
        }
    }

    fn note_masked(&self, count: u64) {
        self.masked_values.fetch_add(count, Ordering::Relaxed);
    }

    /// 引擎失败：pass 模式 = 降级放行（degraded）；block 模式 = 已阻断
    ///（engine_blocked，单独计数，不是降级）。日志文案按模式区分。
    fn note_engine_failure(&self, block: bool) {
        if block {
            self.engine_blocked.fetch_add(1, Ordering::Relaxed);
            log::error!("隐私引擎失败：已按隐私优先阻断本次传输");
        } else {
            self.degraded.fetch_add(1, Ordering::Relaxed);
            log::error!("隐私引擎降级：本次传输按可用性优先放行（未脱敏/未还原）");
        }
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

    /// 流式还原器（共享引擎版；事件级注入点在 restorer 内部）。
    pub fn stream_restorer(&self) -> Option<StreamRestorer<'static>> {
        let engine = self.engine()?;
        #[cfg(feature = "test-hooks")]
        let mut restorer = StreamRestorer::shared(engine);
        #[cfg(not(feature = "test-hooks"))]
        let restorer = StreamRestorer::shared(engine);
        #[cfg(feature = "test-hooks")]
        {
            // 只有 FAULT_STREAM 上膛时才注入；事件号默认 1（第一个事件）
            let event = if self.injected_failure() == FAULT_STREAM {
                let at = self.stream_fault_event();
                if at == 0 {
                    1
                } else {
                    at
                }
            } else {
                0
            };
            restorer.set_fault_event(event);
        }
        Some(restorer)
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

    /// test-hooks：FAULT_STREAM 在第 N 个事件的还原中触发。
    #[cfg(feature = "test-hooks")]
    pub fn set_stream_fault_event(&self, event: u32) {
        self.fault_stream_at.store(event, Ordering::SeqCst);
    }

    #[cfg(feature = "test-hooks")]
    pub fn stream_fault_event(&self) -> u32 {
        self.fault_stream_at.load(Ordering::SeqCst)
    }
}

/// 计数快照。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrivacyStats {
    pub masked_values: u64,
    pub degraded: u64,
    /// block 模式下的引擎失败（已阻断）。
    pub engine_blocked: u64,
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
                // pass：content-encoding 头已要删，不能发回压缩字节——放行解压结果；
                // 解不开的编码例外（保留原头原字节透传）
                gate.note_engine_failure(false);
                bytes.to_vec()
            }
        },
        None => bytes.to_vec(),
    };
    let Ok(text) = String::from_utf8(plain.clone()) else {
        if settings.block {
            return privacy_error(
                StatusCode::BAD_REQUEST,
                "privacy_engine_failed",
                "请求体不是 UTF-8 文本，隐私层无法遮盖。",
            );
        }
        // pass：发解压后的明文字节（与已删除的 content-encoding 头一致）
        gate.note_engine_failure(false);
        return finish_request(next, parts, Bytes::from(plain), &gate, settings).await;
    };
    // 含 base64 图片/文件：隐私优先且未放行 → 阻断；否则继续遮盖
    //（mask_json 对 data: 值原样保留，只遮文本）并计数
    let has_media = contains_unscannable_media(&text);
    if has_media && !settings.allow_unscanned_media && settings.block {
        return privacy_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "privacy_media_blocked",
            "请求含 base64 图片或文件，隐私层无法扫描。可在隐私设置中允许未扫描媒体（allow_unscanned_media）后重试。",
        );
    }
    if has_media {
        gate.note_media_passed();
    }
    let masked = gate.mask_request(&text);
    let payload: Bytes = match masked {
        Ok((masked, _count)) => Bytes::from(masked),
        Err(EngineFailure) => {
            if settings.block {
                gate.note_engine_failure(true);
                return privacy_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "privacy_engine_failed",
                    "隐私引擎遮盖失败，请求已按隐私优先策略阻断。",
                );
            }
            // pass：放行**解压后的明文**（content-encoding 已删，不能发压缩字节）
            gate.note_engine_failure(false);
            Bytes::from(text)
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
            gate.note_engine_failure(settings.block);
            return response;
        }
        let Some(restorer) = gate.stream_restorer() else {
            if settings.block {
                gate.note_engine_failure(true);
                return privacy_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "privacy_engine_failed",
                    "隐私引擎不可用，响应已按隐私优先策略阻断。",
                );
            }
            gate.note_engine_failure(false);
            return response;
        };
        let mut restorer = restorer;
        // 隐私优先：引擎失败后不再放出任何上游字节
        restorer.set_fail_closed(settings.block);
        let (mut parts, body) = response.into_parts();
        let stream = RestoreStream {
            upstream: Box::pin(body.into_data_stream()),
            restorer,
            gate: Arc::clone(gate),
            block: settings.block,
            finished: false,
            pending_error: None,
            failure_noted: false,
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
                // 无法解压的非流式响应不含可还原文本：保留头的压缩语义原样返回
                return Response::from_parts(parts, Body::from(bytes));
            }
        },
        None => bytes.to_vec(),
    };
    let Ok(text) = String::from_utf8(plain.clone()) else {
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
                gate.note_engine_failure(true);
                privacy_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "privacy_engine_failed",
                    "隐私引擎还原失败，响应已按隐私优先策略阻断。",
                )
            } else {
                gate.note_engine_failure(false);
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
    finished: bool,
    /// 已发出其前字节、下一轮再传播的错误（上游错误或隐私优先下的引擎失败）。
    pending_error: Option<std::io::Error>,
    /// 引擎失败已计数（一次生命周期内只计一次）。
    failure_noted: bool,
}

impl RestoreStream {
    /// restorer 报告引擎失败时计数一次；隐私优先返回要传播的错误。
    fn check_failure(&mut self) -> Option<std::io::Error> {
        if !self.restorer.engine_failed() {
            return None;
        }
        if !self.failure_noted {
            self.failure_noted = true;
            self.gate.note_engine_failure(self.block);
        }
        // 头已发出：隐私优先不能正常结束（会被当成成功），以错误断开连接；
        // 可用性优先由 restorer 原样透传。
        self.block
            .then(|| std::io::Error::other("privacy_engine_failed"))
    }

    /// 先发 `out`（非空时），再以 `error` 结束（有错误时）。
    fn emit(
        &mut self,
        out: Vec<u8>,
        error: Option<std::io::Error>,
    ) -> std::task::Poll<Option<Result<Bytes, std::io::Error>>> {
        if error.is_some() {
            self.finished = true;
        }
        match (out.is_empty(), error) {
            (false, error) => {
                self.pending_error = error;
                std::task::Poll::Ready(Some(Ok(Bytes::from(out))))
            }
            (true, Some(error)) => std::task::Poll::Ready(Some(Err(error))),
            (true, None) => std::task::Poll::Ready(None),
        }
    }
}

impl futures::Stream for RestoreStream {
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        // 先传播上一轮积压的错误（其前的字节已发出）
        if let Some(error) = self.pending_error.take() {
            self.finished = true;
            return std::task::Poll::Ready(Some(Err(error)));
        }
        loop {
            if self.finished {
                return std::task::Poll::Ready(None);
            }
            let this = &mut *self;
            match this.upstream.as_mut().poll_next(cx) {
                std::task::Poll::Ready(Some(Ok(chunk))) => {
                    // 引擎 panic 在 restorer 内部逐事件兜住（已还原的照常输出；
                    // 可用性优先从出错处起原始字节透传，隐私优先不再放出上游字节）
                    let out = this.restorer.push(&chunk);
                    if let Some(error) = this.check_failure() {
                        return this.emit(out, Some(error));
                    }
                    if out.is_empty() {
                        continue; // 全部扣留/注释未产出：继续拉
                    }
                    return std::task::Poll::Ready(Some(Ok(Bytes::from(out))));
                }
                std::task::Poll::Ready(Some(Err(error))) => {
                    // 上游错误：先发 finish 的尾部字节，再传播错误
                    let tail = this.restorer.finish();
                    let error = this
                        .check_failure()
                        .unwrap_or_else(|| std::io::Error::other(error));
                    return this.emit(tail, Some(error));
                }
                std::task::Poll::Ready(None) => {
                    // finish 内的还原同样可能失败：隐私优先仍以错误结束
                    let tail = this.restorer.finish();
                    let error = this.check_failure();
                    this.finished = true;
                    return this.emit(tail, error);
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
    let _ = walker::walk(body, &mut |path, key, s| {
        // data: URL；文件字段；Claude 的 source{type:"base64","data":…}
        //（image 与 document 同形）；Gemini 的 inlineData/inline_data.data
        if s.starts_with("data:")
            || matches!(key, "file_data" | "binary_data")
            || key == "data"
                && (path.ends_with(".source.data")
                    || path.ends_with(".inlineData.data")
                    || path.ends_with(".inline_data.data"))
        {
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
