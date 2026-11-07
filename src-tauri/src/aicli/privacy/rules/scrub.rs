//! 归档脱敏（Magpie `scrub.go`，@f11bf27）。
//!
//! 请求存档不保留还原能力：密钥彻底替换为 [REDACTED:KIND]，不生成占位符、
//! 不存值表。长字符串的脱敏结果按值哈希缓存（两代 LRU），避免逐轮重跑全部规则。

use regex::Regex;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// 归档中被替换的值（Magpie `Scrubbed`）。
pub const SCRUBBED: &str = "[REDACTED]";

fn scrubbed(kind: &str) -> String {
    format!("[REDACTED:{kind}]")
}

/// 密钥字段名模式（Magpie `secretName`）。
fn secret_name_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)password|passwd|secret|token|api[_-]?key|access[_-]?key|private[_-]?key|credential|authori[sz]ation|cookie|^key$",
        )
        .unwrap()
    })
}

/// 字段或头的名称是否密钥类（Magpie `SecretName`）。
pub fn secret_name(name: &str) -> bool {
    let n = name.to_lowercase();
    secret_name_re().is_match(&n) && !n.ends_with("tokens") && !n.ends_with("token_count")
}

/// 缓存条目。
struct ScrubEntry {
    gen: u64,
    same: bool,
    out: String,
}

const SCRUB_MEMO_MIN: usize = 256;
const SCRUB_MEMO_ENTRIES: usize = 1 << 15;
const SCRUB_MEMO_BYTES: usize = 32 << 20;

struct Memo {
    cur: HashMap<[u8; 32], ScrubEntry>,
    old: HashMap<[u8; 32], ScrubEntry>,
    bytes: usize,
}

fn memo() -> &'static Mutex<Memo> {
    static M: OnceLock<Mutex<Memo>> = OnceLock::new();
    M.get_or_init(|| {
        Mutex::new(Memo {
            cur: HashMap::new(),
            old: HashMap::new(),
            bytes: 0,
        })
    })
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(bytes);
    h.finalize().into()
}

/// ScrubHead：头的值脱敏（Magpie `ScrubHeader`）。
pub fn scrub_header(name: &str, value: &str) -> String {
    let v = value.trim().to_lowercase();
    if secret_name(name) || v.starts_with("bearer ") || v.starts_with("basic ") {
        return SCRUBBED.to_string();
    }
    // 完整脱敏需要引擎；此处仅按头名称处理
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_names_detected() {
        assert!(secret_name("Authorization"));
        assert!(secret_name("x-api-key"));
        assert!(secret_name("Cookie"));
        assert!(secret_name("client_secret"));
        assert!(secret_name("DB_PASSWORD"));
        assert!(!secret_name("max_tokens"));
        assert!(!secret_name("token_count"));
        assert!(!secret_name("Content-Type"));
    }

    #[test]
    fn scrubbed_format() {
        assert_eq!(scrubbed("API_KEY"), "[REDACTED:API_KEY]");
    }

    #[test]
    fn scrub_header_replaces_whole() {
        assert_eq!(scrub_header("Authorization", "Bearer sk-abc123"), SCRUBBED);
        assert_eq!(scrub_header("x-api-key", "anything"), SCRUBBED);
        assert_eq!(
            scrub_header("Accept", "application/json"),
            "application/json"
        );
    }
}
