//! Deep link URL parser
//!
//! 只解析 `aicliapi://v1/bind`：深链只负责打开绑定窗口，不携带任何密钥（方案 6.2）。
//! 除可选的 `line`（线路 id，只认注册表里的固定值）外，任何查询参数都被拒绝，
//! 防止 `accessKey`、`apiKey`、`ticket` 之类的值进入日志或前端。
//! 错误信息只写参数名，不回显参数值。

use super::BindRequest;
use crate::error::AppError;
use url::Url;

/// Parse a `aicliapi://v1/bind` URL into a [`BindRequest`].
///
/// Expected format: `aicliapi://v1/bind` or `aicliapi://v1/bind?line={line_id}`
pub fn parse_deeplink_url(url_str: &str) -> Result<BindRequest, AppError> {
    // Parse URL
    let url = Url::parse(url_str)
        .map_err(|e| AppError::InvalidInput(format!("Invalid deep link URL: {e}")))?;

    // Validate scheme
    let scheme = url.scheme();
    if scheme != "aicliapi" {
        return Err(AppError::InvalidInput(format!(
            "Invalid scheme: expected 'aicliapi', got '{scheme}'"
        )));
    }

    // Extract version from host
    let version = url
        .host_str()
        .ok_or_else(|| AppError::InvalidInput("Missing version in URL host".to_string()))?
        .to_string();

    // Validate version
    if version != "v1" {
        return Err(AppError::InvalidInput(format!(
            "Unsupported protocol version: {version}"
        )));
    }

    // Extract path (should be "/bind")
    let path = url.path();
    if path != "/bind" {
        return Err(AppError::InvalidInput(format!(
            "Invalid path: expected '/bind', got '{path}'"
        )));
    }

    // 只允许一个可选的 `line` 参数；带任何其他参数（包括 `accessKey`、`apiKey`、
    // `ticket`）都拒绝。逐对遍历而不是收进 HashMap，重复的 `line` 也拒绝。
    let mut line: Option<String> = None;
    for (key, value) in url.query_pairs() {
        if key != "line" {
            return Err(AppError::InvalidInput(format!(
                "Unsupported query parameter: '{key}'"
            )));
        }
        if line.is_some() {
            return Err(AppError::InvalidInput(
                "Duplicate 'line' parameter".to_string(),
            ));
        }
        // 线路 id 只认 `aicli::lines` 注册表里的固定值；不回显收到的值
        if crate::aicli::lines::line(value.as_ref()).is_none() {
            return Err(AppError::InvalidInput(
                "Parameter 'line' is not a known line id".to_string(),
            ));
        }
        line = Some(value.into_owned());
    }

    Ok(BindRequest { version, line })
}
