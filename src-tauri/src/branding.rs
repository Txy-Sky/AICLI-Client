//! 产品身份（清单 0.4 节固定取值）：界面文案、外部链接、User-Agent 统一从这里取，
//! 不再写死上游 cc-switch 的名字和地址。

/// 产品名（各语言相同）。
pub const PRODUCT_NAME: &str = "AICLI Client";
/// 官网。
pub const WEBSITE_URL: &str = "https://aicliapi.com";
/// 项目仓库与发布页（v1 不发布 Release，updater 已关闭；链接只作为入口）。
pub const REPOSITORY_URL: &str = "https://github.com/Txy-Sky/AICLI-Client";
pub const RELEASES_URL: &str = "https://github.com/Txy-Sky/AICLI-Client/releases";
/// 对外请求的 User-Agent。
pub const USER_AGENT: &str = concat!("aicli-client/", env!("CARGO_PKG_VERSION"));
/// 应用诊断日志文件名（与 lib.rs 里日志插件的 file_name 一致）。
pub const LOG_FILE_NAME: &str = "aicli-client.log";
