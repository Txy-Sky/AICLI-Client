//! E-03 检测规则：Magpie `internal/redact/` 的移植。
//!
//! 移植自 github.com/yetone/magpie @ f11bf27c1f3f0f7d5284dffa2cb8b066debaed84
//! （MIT，见仓库根 THIRD_PARTY_NOTICES.md）。`model.go` 不移植（v1 无本地模型）；
//! `zhname.go`/`zhdict.txt` 留待 E-08。
//!
//! Go→Rust 迁移约定（全模块一致）：
//! - Go 字符串按字节索引；Rust 侧统一用 `s.as_bytes()[i]` 检查字节、只在字符边界
//!   切子串（正则匹配端点必为边界，回看的字节检查用 as_bytes）。
//! - Go regexp（RE2）与 Rust `regex` crate 语法兼容；个别语义差异（如 RE2 的
//!   `[[:alpha:]]`、无反引用限制）在出现处以注释注明。
//! - Go 的 `strings.EqualFold` 仅用于 ASCII 场景时以 `eq_ignore_ascii_case` 对应。

#![allow(dead_code, clippy::module_inception)] // E-03 WIP：各叶子模块在 redact.rs 引擎接线前暂无调用方

pub mod known;
pub mod redact;
pub mod rules; // 名字与父模块同名是刻意的：对齐 Magpie rules.go
pub mod scrub;
pub mod shape;
pub mod zhname;

pub mod dateword;
pub mod fieldvalue;
pub mod ids;
pub mod more;
pub mod phone;

/// 找到的一段值（Magpie `span`）：字节区间 [start, end)。
/// 引擎（redact.rs）与各规则模块使用；当前批次尚未接线的字段允许未用。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Span {
    pub start: usize,
    pub end: usize,
    pub kind: &'static str,
    pub by: FoundBy,
    pub bare: bool,
}

/// 哪个发现器找到的（Magpie `By*` 常量）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FoundBy {
    Rule,
    Yours,
    Word,
    Model,
    Known,
}

/// 字符集常量（Magpie `alnum`/`tokenCh`/`digits`）。
pub(crate) const ALNUM: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
pub(crate) const TOKEN_CH: &[u8] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";
pub(crate) const DIGITS: &[u8] = b"0123456789";

pub(crate) fn in_set(set: &[u8], c: u8) -> bool {
    set.contains(&c)
}

/// 词字节：字母、数字或 _（Magpie `wordByte`）。
pub(crate) fn word_byte(s: &[u8], i: usize) -> bool {
    let c = s[i];
    c == b'_' || c.is_ascii_alphanumeric()
}

/// 字段的名称切成小写词（Magpie `nameWords`）：next_page_token、nextPageToken、
/// X-Next-Page-Token → next, page, token。
pub(crate) fn name_words(name: &str) -> Vec<String> {
    let bytes = name.as_bytes();
    let mut out = Vec::new();
    let mut word: Vec<u8> = Vec::new();
    let flush = |word: &mut Vec<u8>, out: &mut Vec<String>| {
        if !word.is_empty() {
            out.push(String::from_utf8_lossy(word).into_owned());
            word.clear();
        }
    };
    for i in 0..bytes.len() {
        let c = bytes[i];
        if c == b'_' || c == b'-' || c == b'.' {
            flush(&mut word, &mut out);
        } else if c.is_ascii_uppercase() {
            if i > 0 && bytes[i - 1].is_ascii_lowercase() {
                flush(&mut word, &mut out);
            }
            word.push(c - b'A' + b'a');
        } else {
            word.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// 值起始处 s[at] 前面字段名的词（Magpie `fieldWords`）：冒号或等号之前。
pub(crate) fn field_words(s: &[u8], at: usize) -> Vec<String> {
    let mut i = at;
    while i > 0 && b" \t\"'\\".contains(&s[i - 1]) {
        i -= 1;
    }
    if i == 0 || (s[i - 1] != b':' && s[i - 1] != b'=') {
        return Vec::new();
    }
    i -= 1;
    while i > 0 && b" \t\"'\\".contains(&s[i - 1]) {
        i -= 1;
    }
    let mut j = i;
    while j > 0 && (in_set(ALNUM, s[j - 1]) || b"_.".contains(&s[j - 1])) {
        j -= 1;
    }
    name_words(&String::from_utf8_lossy(&s[j..i]))
}

/// 是否含 CJK 汉字（Magpie `hasCJK`）。
pub(crate) fn has_cjk(s: &str) -> bool {
    s.chars().any(|r| ('\u{4e00}'..='\u{9fff}').contains(&r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_words_split() {
        assert_eq!(name_words("next_page_token"), ["next", "page", "token"]);
        assert_eq!(name_words("nextPageToken"), ["next", "page", "token"]);
        assert_eq!(
            name_words("X-Next-Page-Token"),
            ["x", "next", "page", "token"]
        );
        assert_eq!(name_words("phone"), ["phone"]);
        assert!(name_words("").is_empty());
    }

    #[test]
    fn word_byte_set() {
        let s = b"a_1-";
        assert!(word_byte(s, 0));
        assert!(word_byte(s, 1));
        assert!(word_byte(s, 2));
        assert!(!word_byte(s, 3));
    }
}
