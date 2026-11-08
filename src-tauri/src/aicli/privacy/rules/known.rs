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
    /// 种类（驻留为 'static，供 Span.kind 使用；再遮时得到同一占位符）。
    pub kind: &'static str,
}

/// 种类名驻留：Span.kind 是 &'static str，种类来自占位符字符串。种类集合
/// 有限（规则种类 + 自定义词条），每种只泄漏一次。
fn intern_kind(kind: &str) -> &'static str {
    static KINDS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    let mut kinds = KINDS.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(k) = kinds.iter().find(|k| **k == kind) {
        return k;
    }
    let leaked: &'static str = Box::leak(kind.to_owned().into_boxed_str());
    kinds.push(leaked);
    leaked
}

/// 已知值索引（Magpie `known`/`knownBits`/`knownGen`）。
pub(crate) struct KnownIndex {
    /// 前 MIN_KNOWN 字节 → 已知值列表。按字节作键（Go 的 value[:8] 是
    /// 字节切片）：中文值的第 8 字节常落在字符中间，按 &str 切会 panic。
    by_head: HashMap<Vec<u8>, Vec<KnownValue>>,
    /// 4 字节 gram 位图（Magpie `knownBits`）
    bits: Vec<u64>,
    /// 变化世代（Magpie `knownGen`）
    pub generation: AtomicU64,
}

const BITS_LEN: usize = 1 << 14; // 2^20 位 → 2^14 个 u64

fn gram(s: &[u8], i: usize) -> usize {
    let g = u32::from_le_bytes([s[i], s[i + 1], s[i + 2], s[i + 3]]);
    // 与 Go uint32 乘法一致：回绕后右移 12 位 → 最多 20 位
    (g.wrapping_mul(2654435761) >> 12) as usize
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
        let head = &value.as_bytes()[..MIN_KNOWN];
        let existing = self.by_head.entry(head.to_vec()).or_default();
        for k in existing.iter() {
            if k.value == value {
                return; // 已记录
            }
        }
        existing.push(KnownValue {
            value: value.to_string(),
            placeholder: placeholder.to_string(),
            kind: intern_kind(kind),
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
            let head = &bytes[i..i + MIN_KNOWN];
            let Some(candidates) = self.by_head.get(head) else {
                i += 1;
                continue;
            };
            let mut best: Option<&KnownValue> = None;
            for k in candidates {
                if !covers_kind(k.kind) {
                    continue;
                }
                // 按字节比较：值是合法 UTF-8，命中处必然落在字符边界上
                if !bytes[i..].starts_with(k.value.as_bytes()) {
                    continue;
                }
                // Magpie knownSpans：只认独立出现的值，不遮更长内容里的片段
                if !stands_alone(k.kind, bytes, i, i + k.value.len()) {
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
                    kind: b.kind,
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
        _ => {
            (!is(before, alnum) || !is(s[a], alnum)) && (!is(after, alnum) || !is(s[b - 1], alnum))
        }
    }
}

/// 共享的已知值索引（进程级，Magpie 的全局 `known`）。
#[cfg(not(test))]
pub(crate) fn shared_index() -> &'static Mutex<KnownIndex> {
    static IDX: std::sync::OnceLock<Mutex<KnownIndex>> = std::sync::OnceLock::new();
    IDX.get_or_init(|| Mutex::new(KnownIndex::new()))
}

/// 单元测试构建：每个线程一份（`cargo test` 一测试一线程），避免一个测试
/// 遮过的值在另一个测试里被再匹配；与 CI 的 nextest（一测试一进程）一致。
/// 每线程泄漏一个索引，只在测试里发生。
#[cfg(test)]
pub(crate) fn shared_index() -> &'static Mutex<KnownIndex> {
    thread_local! {
        static IDX: &'static Mutex<KnownIndex> =
            Box::leak(Box::new(Mutex::new(KnownIndex::new())));
    }
    IDX.with(|idx| *idx)
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

    /// 中文值：第 8 字节落在字符中间，按字节索引不 panic；中文正文里
    /// 每个字节位置都试探一遍也不 panic。
    #[test]
    fn cjk_values_index_by_bytes() {
        let name = ["司马", "鸿远"].concat();
        let idx = build(&[("{{PERSON_k7f3q9xa}}", &name)]);
        let text = format!("这段代码是{name}写的，{name}。");
        let spans = idx.spans(&text, covers_all);
        assert_eq!(spans.len(), 2);
        for sp in &spans {
            assert_eq!(&text[sp.start..sp.end], name);
        }
        assert!(idx.spans("司马光砸缸的故事很长很长", covers_all).is_empty());
    }

    /// 再匹配带出原种类（再遮得到同一占位符），且只认独立出现的值
    /// （Magpie knownSpans 的 standsAlone）。
    #[test]
    fn spans_carry_kind_and_require_standing_alone() {
        let pw = ["Qw7m", "Pz3kLx9v"].concat();
        let idx = build(&[("{{PASSWORD_ab2cd3ef}}", &pw)]);
        let spans = idx.spans(&format!("参数 {pw} 已更新"), covers_all);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].kind, "PASSWORD");
        assert_eq!(spans[0].by, FoundBy::Known);
        // 更长字母数字串里的片段不算
        assert!(idx.spans(&format!("x{pw}9"), covers_all).is_empty());
        assert!(idx.spans(&format!("{pw}Z"), covers_all).is_empty());
    }
}
