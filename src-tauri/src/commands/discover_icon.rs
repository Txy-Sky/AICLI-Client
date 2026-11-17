//! 发现栏卡片图标的本地缓存。
//!
//! 界面先直接从图标地址（GitHub 头像、MCP Registry 给的图标等）加载；加载成功后调
//! `cache_discover_icon` 由这里下载一份存进 `~/.aicli-client/cache/discover-icons/`，
//! 下次打开先用 `get_cached_discover_icon` 读本地，不再请求网络。
//! 只接受 https、不带账号密码的地址；内容按文件头识别，只存常见图片格式（含 SVG），
//! 单个最大 1MB。返回 data URL 给 `<img>` 用（SVG 在 `<img>` 里不执行脚本）。

use std::path::PathBuf;

use sha2::{Digest, Sha256};

const MAX_ICON_BYTES: usize = 1024 * 1024;
const MAX_URL_LEN: usize = 2048;
const DOWNLOAD_TIMEOUT_SECS: u64 = 20;

fn cache_dir() -> PathBuf {
    crate::config::get_app_config_dir()
        .join("cache")
        .join("discover-icons")
}

fn cache_key(url: &str) -> String {
    Sha256::digest(url.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn validate_icon_url(raw: &str) -> Result<url::Url, String> {
    if raw.is_empty() || raw.len() > MAX_URL_LEN {
        return Err("invalid icon url".to_string());
    }
    let parsed = url::Url::parse(raw).map_err(|_| "invalid icon url".to_string())?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("icon url must be https without credentials".to_string());
    }
    Ok(parsed)
}

/// 按文件头识别图片类型；不是图片返回 None（不信响应头，免得把网页当图片存下）
fn sniff_image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if bytes.starts_with(&[0x00, 0x00, 0x01, 0x00]) {
        return Some("image/x-icon");
    }
    let head = &bytes[..bytes.len().min(1024)];
    let text = String::from_utf8_lossy(head).to_ascii_lowercase();
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    if (trimmed.starts_with("<svg") || trimmed.starts_with("<?xml")) && text.contains("<svg") {
        return Some("image/svg+xml");
    }
    None
}

fn data_url(mime: &str, bytes: &[u8]) -> String {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    format!("data:{mime};base64,{}", STANDARD.encode(bytes))
}

/// 读本地缓存：有就返回 data URL，没有（或文件已不是图片）返回 None
#[tauri::command]
pub async fn get_cached_discover_icon(url: String) -> Result<Option<String>, String> {
    validate_icon_url(&url)?;
    let path = cache_dir().join(cache_key(&url));
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(_) => return Ok(None),
    };
    Ok(sniff_image_type(&bytes).map(|mime| data_url(mime, &bytes)))
}

/// 下载图标存进本地缓存（已存在就不再下载）。成功返回 true。
#[tauri::command]
pub async fn cache_discover_icon(url: String) -> Result<bool, String> {
    let parsed = validate_icon_url(&url)?;
    let dir = cache_dir();
    let path = dir.join(cache_key(&url));
    if tokio::fs::try_exists(&path).await.unwrap_or(false) {
        return Ok(true);
    }

    let client = crate::proxy::http_client::get();
    let mut resp = client
        .get(parsed)
        .header("Accept", "image/*")
        .timeout(std::time::Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        if body.len() + chunk.len() > MAX_ICON_BYTES {
            return Err("icon too large".to_string());
        }
        body.extend_from_slice(&chunk);
    }
    if sniff_image_type(&body).is_none() {
        return Err("not an image".to_string());
    }

    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| e.to_string())?;
    // 先写临时文件再改名，避免读到写了一半的文件
    let tmp = dir.join(format!("{}.tmp", cache_key(&url)));
    tokio::fs::write(&tmp, &body)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::rename(&tmp, &path)
        .await
        .map_err(|e| e.to_string())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_urls_without_credentials_are_accepted() {
        assert!(validate_icon_url("https://github.com/acme.png?size=96").is_ok());
        assert!(validate_icon_url("http://github.com/acme.png").is_err());
        assert!(validate_icon_url("https://user:pw@example.com/a.png").is_err());
        assert!(validate_icon_url("file:///etc/passwd").is_err());
        assert!(validate_icon_url("").is_err());
        assert!(validate_icon_url(&format!("https://a.dev/{}", "x".repeat(3000))).is_err());
    }

    #[test]
    fn images_are_recognized_by_their_bytes() {
        assert_eq!(
            sniff_image_type(b"\x89PNG\r\n\x1a\nrest"),
            Some("image/png")
        );
        assert_eq!(
            sniff_image_type(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some("image/jpeg")
        );
        assert_eq!(sniff_image_type(b"GIF89a...."), Some("image/gif"));
        assert_eq!(
            sniff_image_type(b"RIFF\0\0\0\0WEBPVP8 "),
            Some("image/webp")
        );
        assert_eq!(
            sniff_image_type(b"<?xml version=\"1.0\"?><svg xmlns=\"x\"></svg>"),
            Some("image/svg+xml")
        );
        assert_eq!(
            sniff_image_type(b"  <svg viewBox=\"0 0 1 1\"/>"),
            Some("image/svg+xml")
        );
        assert_eq!(sniff_image_type(b"<!doctype html><html>"), None);
        assert_eq!(sniff_image_type(b"{\"error\":1}"), None);
    }

    #[test]
    fn cache_keys_are_stable_hex_names() {
        let key = cache_key("https://github.com/acme.png?size=96");
        assert_eq!(key.len(), 64);
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(key, cache_key("https://github.com/acme.png?size=96"));
    }

    #[test]
    fn data_urls_carry_the_type() {
        assert_eq!(data_url("image/png", b"ab"), "data:image/png;base64,YWI=");
    }
}
