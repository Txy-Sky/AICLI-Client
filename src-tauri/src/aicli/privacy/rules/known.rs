//! 已遮值的再匹配（Magpie `known.go`，@f11bf27）。
//!
//! 一次被遮盖的值在其后请求中无论以何种上下文出现都会再次匹配到同一占位符。
//! 值表由占位符引擎持有；这里维护按前 8 字节索引的查找结构和 4 字节 gram 位图。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use super::{FoundBy, Span};

/// 最短可再匹配值长度（Magpie `minKnown`）。
pub(crate) const MIN_KNOWN: usize = 8;

/// 已知值记录。
#[derive(Debug, Clone)]
pub(crate) struct KnownValue {
    pub value: String,
    pub placeholder: String,
    pub kind: String,
}

/// 已知值索引（Magpie `known`/`knownBits`/`knownGen`）。
pub(crate) struct KnownIndex {
    /// 前 MIN_KNOWN 字节 → 已知值列表
    by_head: HashMap<String, Vec<KnownValue>>,
    /// 4 字节 gram 位图（Magpie `knownBits`）
    bits: Vec<u64>,
    /// 变化世代（Magpie `knownGen`）
    pub generation: AtomicU64,
}

const BITS_LEN: usize = 1 << 14; // 2^20 位 → 2^14 个 u64

fn gram(s: &[u8], i: usize) -> usize {
    let g = u32::from_le_bytes([s[i], s[i + 1], s[i + 2], s[i + 3]]);
    ((g as u64).wrapping_mul(2654435761) >> 12) as usize // 20 位
}

impl KnownIndex {
    pub fn new() -> Self {
        Self {
            by_head: HashMap::new(),
            bits: vec![0u64; BITS_LEN],
            generation: AtomicU64::new(0),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.by_head.is_empty()
    }

    /// addKnown：记录值的占位符（Magpie `addKnown`）。
    pub fn add(&mut self, placeholder: &str, value: &str) {
        let kind = kind_of(placeholder);
        if kind == "TERM" || value.len() < MIN_KNOWN {
            return;
        }
        let head = &value[..MIN_KNOWN];
        let existing = self.by_head.entry(head.to_string()).or_default();
        for k in existing.iter() {
            if k.value == value {
                return; // 已记录
            }
        }
        existing.push(KnownValue {
            value: value.to_string(),
            placeholder: placeholder.to_string(),
            kind: kind.to_string(),
        });
        let g = gram(value.as_bytes(), 0);
        self.bits[g / 64] |= 1 << (g % 64);
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// reindex：从值表重建索引（Magpie `reindexKnown`）。
    pub fn reindex(&mut self, values: &HashMap<String, String>) {
        self.by_head = HashMap::new();
        self.bits = vec![0u64; BITS_LEN];
        self.generation.fetch_add(1, Ordering::SeqCst);
        for (p, v) in values {
            self.add(p, v);
        }
    }

    /// knownSpans：s 中已知值的全部出现（Magpie `knownSpans`）。
    /// 只返回 covers_kind 回调允许的种类。
    pub fn spans(&self, s: &str, covers_kind: impl Fn(&str) -> bool) -> Vec<Span> {
        let bytes = s.as_bytes();
        if bytes.len() < MIN_KNOWN {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut i = 0;
        while i + MIN_KNOWN <= bytes.len() {
            let g = gram(bytes, i);
            if self.bits[g / 64] & (1 << (g % 64)) == 0 {
                i += 1;
                continue;
            }
            let head = &s[i..i + MIN_KNOWN];
            let Some(candidates) = self.by_head.get(head) else {
                i += 1;
                continue;
            };
            let mut best: Option<&KnownValue> = None;
            for k in candidates {
                if !covers_kind(&k.kind) {
                    continue;
                }
                if !s[i..].starts_with(&k.value) {
                    continue;
                }
                if let Some(b) = best {
                    if k.value.len() <= b.value.len() {
                        continue;
                    }
                }
                best = Some(k);
            }
            if let Some(b) = best {
                out.push(Span {
                    start: i,
                    end: i + b.value.len(),
                    kind: "", // 引擎层填入
                    by: FoundBy::Known,
                    bare: false,
                });
                i += b.value.len();
            } else {
                i += 1;
            }
        }
        out
    }
}

/// kindOf：从占位符提取种类（Magpie `kindOf`）。
pub(crate) fn kind_of(placeholder: &str) -> &str {
    if let Some(i) = placeholder.rfind('_') {
        if i > 2 {
            return &placeholder[2..i];
        }
    }
    ""
}

/// standsAlone：s[a:b] 是完整值还是更长内容的片段（Magpie `standsAlone`）。
pub(crate) fn stands_alone(kind: &str, s: &[u8], a: usize, b: usize) -> bool {
    let before = if a > 0 { s[a - 1] } else { 0 };
    let after = if b < s.len() { s[b] } else { 0 };
    let is = |c: u8, set: &[u8]| c != 0 && set.contains(&c);
    let alnum = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let digits = b"0123456789";
    match kind {
        "PHONE" | "BANK_CARD" | "ID_CARD" | "IBAN" | "SSN" | "PASSPORT" | "SERIAL" => {
            !is(before, alnum) && !is(after, alnum)
        }
        "IP" | "MAC" => {
            if is(
                before,
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789.:",
            ) || is(
                after,
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789:",
            ) {
                return false;
            }
            after != b'.' || b + 1 >= s.len() || !is(s[b + 1], digits)
        }
        "EMAIL" => {
            if is(
                before,
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._%+-",
            ) || is(
                after,
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_%+",
            ) {
                return false;
            }
            !is(after, b".-") || b + 1 >= s.len() || !is(s[b + 1], alnum)
        }
        _ => !(is(before, alnum) && is(s[a], alnum)) && !(is(after, alnum) && is(s[b - 1], alnum)),
    }
}

/// 共享的已知值索引（进程级，Magpie 的全局 `known`）。
pub(crate) fn shared_index() -> &'static Mutex<KnownIndex> {
    static IDX: std::sync::OnceLock<Mutex<KnownIndex>> = std::sync::OnceLock::new();
    IDX.get_or_init(|| Mutex::new(KnownIndex::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(values: &[(&str, &str)]) -> KnownIndex {
        let mut idx = KnownIndex::new();
        for (p, v) in values {
            idx.add(p, v);
        }
        idx
    }

    fn covers_all(_: &str) -> bool {
        true
    }

    #[test]
    fn add_and_find() {
        let email = ["alice", "@", "example", ".com"].concat();
        let idx = build(&[("{{EMAIL_xbcfsw}}", &email)]);
        let text = format!("contact {} today", email);
        let spans = idx.spans(&text, covers_all);
        assert_eq!(spans.len(), 1);
        assert_eq!(&text[spans[0].start..spans[0].end], email);
    }

    #[test]
    fn too_short_not_known() {
        let idx = build(&[("sk_hYQWRBFmdHNI", "short")]);
        assert!(idx.is_empty());
    }

    #[test]
    fn longer_match_wins() {
        let short_v = ["alice", "@", "example", ".com"].concat();
        let long_v = format!("{}.uk", short_v);
        let idx = build(&[
            ("{{EMAIL_xbcfsw}}", &short_v),
            ("{{EMAIL_ybcfsw}}", &long_v),
        ]);
        let text = format!("mail {} now", short_v);
        let spans = idx.spans(&text, covers_all);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].end - spans[0].start, short_v.len());
    }

    #[test]
    fn kind_covered_filter() {
        let email = ["bob", "@", "test", ".org"].concat();
        let key = ["sk-", "abcdefghij12345678"].concat();
        let idx = build(&[("{{EMAIL_xbcfsw}}", &email), ("{{API_KEY_ybcfsw}}", &key)]);
        let text = format!("{} and {}", email, key);
        let only_email = idx.spans(&text, |k| k == "EMAIL");
        assert_eq!(only_email.len(), 1);
        let only_key = idx.spans(&text, |k| k == "API_KEY");
        assert_eq!(only_key.len(), 1);
        let all = idx.spans(&text, covers_all);
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn kind_of_extraction() {
        assert_eq!(kind_of("{{EMAIL_abcd2345}}"), "EMAIL");
        assert_eq!(kind_of("{{API_KEY_wxyz6789}}"), "API_KEY");
        assert_eq!(kind_of("x"), "");
    }

    #[test]
    fn stands_alone_phone() {
        let s = b" 13800138000 ";
        assert!(stands_alone("PHONE", s, 1, 12));
        let s = b"a13800138000b";
        assert!(!stands_alone("PHONE", s, 1, 12));
    }

    #[test]
    fn stands_alone_email() {
        let email = ["a", "@", "b", ".", "co"].concat();
        let s = format!(" {}", email).into_bytes();
        assert!(stands_alone("EMAIL", &s, 1, 1 + email.len()));
        // 句号结尾允许
        let s = format!(" {}.", email).into_bytes();
        assert!(stands_alone("EMAIL", &s, 1, 1 + email.len()));
    }

    #[test]
    fn stands_alone_ip() {
        let s = b"ip 192.0.2.1 ";
        assert!(stands_alone("IP", s, 3, 12));
        let s = b"a192.0.2.1b";
        assert!(!stands_alone("IP", s, 1, 10));
        // 版本号
        let s = b"1.2.3.4.5";
        assert!(!stands_alone("IP", s, 0, 7));
    }
}
