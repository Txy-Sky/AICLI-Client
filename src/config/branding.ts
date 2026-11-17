/**
 * 产品身份（清单 0.4 节固定取值，与 src-tauri/src/branding.rs 一致）：界面文案、
 * 外部链接统一从这里取，不再写死上游 cc-switch 的名字和地址。
 */
export const PRODUCT_NAME = "AICLI Client";
export const WEBSITE_URL = "https://aicliapi.com";
export const REPOSITORY_URL = "https://github.com/Txy-Sky/AICLI-Client";
export const RELEASES_URL = "https://github.com/Txy-Sky/AICLI-Client/releases";
/** 配置目录（~ 下）。 */
export const APP_DIR_NAME = ".aicli-client";
/** WebDAV / S3 同步的默认远端目录（不与原 cc-switch 的 cc-switch-sync 共用）。 */
export const DEFAULT_SYNC_REMOTE_ROOT = "aicli-client-sync";
