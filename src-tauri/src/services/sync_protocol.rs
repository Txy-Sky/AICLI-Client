//! 数据库快照导入/导出的互斥锁。
//!
//! （G-04 2026-11-10：WebDAV / S3 云同步已移除——它们会把含令牌的数据库同步到外部，
//! 与 AICLI 的凭据本机化策略冲突。原 `sync_protocol.rs` 的传输协议、清单与打包逻辑
//! 随之删除，只保留这把锁：SQL 导入会整库替换，与提示词写入、快照恢复互斥。）

use std::sync::OnceLock;

/// 串行化所有会整库替换或快照恢复的操作。
pub(crate) fn sync_mutex() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}
