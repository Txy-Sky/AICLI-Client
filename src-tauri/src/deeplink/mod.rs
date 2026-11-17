//! Bind deep link for AICLI Client
//!
//! `aicliapi://v1/bind` 深链只负责打开绑定窗口，不携带任何密钥（方案 6.2）。
//! 解析成功后把结果暂存为待处理深链接并发 `aicli-open-bind` 事件（空负载），
//! 由前端打开绑定窗口；解析失败只记日志，不暂存任何链接内容。

mod parser;
#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

// Re-export public API
pub use parser::parse_deeplink_url;

/// 解析后的绑定深链请求
///
/// 代表一条已通过校验的 `aicliapi://v1/bind` URL；`line` 若存在，
/// 只可能是 `aicli::lines` 注册表里的线路 id。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BindRequest {
    /// Protocol version (e.g., "v1")
    pub version: String,
    /// Optional line id from the fixed registry (`main` | `global` | `apac`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
}

/// 等前端处理的深链接（解析结果）
///
/// 链接经常在前端还没注册监听时到达：冷启动、轻量模式重建窗口。事件一发出去就没了，
/// 所以先存在这里，再发 `aicli-open-bind` 通知；前端在挂载时和收到通知时都调
/// `peek_pending_deeplink` 读一次，真正打开绑定窗口之后再按 `id` 调
/// `ack_pending_deeplink` 清除。
///
/// 读取不清除：前端读到之后、打开窗口之前可能被打断（启动时校正语言会重建 effect、
/// 窗口被销毁），读即清的话链接就丢了。同一条链接收到多次通知，前端按 `id` 去重。
/// 解析失败的链接不会进入这里：只写日志，不暂存（修复旧实现把原始 URL 存内存并交给
/// 前端的缺陷）。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PendingDeepLink {
    Bind { request: Box<BindRequest> },
}

/// 带编号的待处理深链接，编号用于确认清除和前端去重
#[derive(Debug, Clone, Serialize)]
pub struct PendingDeepLinkEntry {
    pub id: u64,
    pub link: PendingDeepLink,
}

static PENDING_DEEPLINK: Mutex<Option<PendingDeepLinkEntry>> = Mutex::new(None);
static NEXT_PENDING_ID: AtomicU64 = AtomicU64::new(1);

fn pending_slot() -> std::sync::MutexGuard<'static, Option<PendingDeepLinkEntry>> {
    PENDING_DEEPLINK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 暂存一条待处理的深链接，覆盖还没处理的旧链接（只保留最新一条）
pub fn set_pending_deeplink(link: PendingDeepLink) {
    let id = NEXT_PENDING_ID.fetch_add(1, Ordering::Relaxed);
    *pending_slot() = Some(PendingDeepLinkEntry { id, link });
}

/// 读取待处理的深链接，不清除
pub fn peek_pending_deeplink() -> Option<PendingDeepLinkEntry> {
    pending_slot().clone()
}

/// 前端处理之后确认清除；编号对不上（期间来了新链接）就不动
pub fn ack_pending_deeplink(id: u64) {
    let mut slot = pending_slot();
    if slot.as_ref().is_some_and(|entry| entry.id == id) {
        *slot = None;
    }
}
