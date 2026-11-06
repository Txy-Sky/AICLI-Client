//! id 形状判定（Magpie `ids.go`，@f11bf27）。
//!
//! 后端生成的 id（UUID、≥32 位十六进制）原样放行；只有整体或作为凭据字段的
//! 值才遮盖，绝不允许遮一半。

use super::{field_words, word_byte, FoundBy, Span};

/// 元素 token 字段（cua-driver）：element + token 两词。
fn element_field(words: &[String]) -> bool {
    words.len() == 2 && words[0] == "element" && words[1] == "token"
}

/// s[a:b] 是否 cua-driver 的元素 token（Magpie `elementToken`）。
pub(crate) fn element_token(s: &[u8], a: usize, b: usize) -> bool {
    if b - a < 9 || s[a] != b's' || a > 0 && word_byte(s, a - 1) {
        return false;
    }
    for &c in &s[a + 1..a + 9] {
        if !(c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) {
            return false;
        }
    }
    let mut end = a + 9;
    if end < s.len() && s[end] == b':' {
        let mut i = end + 1;
        while i < s.len() && s[i].is_ascii_digit() {
            i += 1;
        }
        if i == end + 1 {
            return false; // 冒号后无索引
        }
        end = i;
    }
    if (b != a + 9 && b != end) || end < s.len() && word_byte(s, end) {
        return false;
    }
    end > a + 9 || element_field(&field_words(s, a))
}

fn hex_or_hyphen(c: u8) -> bool {
    c == b'-' || c.is_ascii_digit() || (b'a'..=b'f').contains(&c) || (b'A'..=b'F').contains(&c)
}

/// UUID 正则（Magpie `uuidRe`）。
pub(crate) fn is_uuid(v: &str) -> bool {
    // 8-4-4-4-12 十六进制；RE2 与 regex 语法一致
    let b = v.as_bytes();
    v.len() == 36
        && b[..8].iter().all(|&c| hex_or_hyphen_nohyphen(c))
        && b[8] == b'-'
        && b[9..13].iter().all(|&c| hex_or_hyphen_nohyphen(c))
        && b[13] == b'-'
        && b[14..18].iter().all(|&c| hex_or_hyphen_nohyphen(c))
        && b[18] == b'-'
        && b[19..23].iter().all(|&c| hex_or_hyphen_nohyphen(c))
        && b[23] == b'-'
        && b[24..].iter().all(|&c| hex_or_hyphen_nohyphen(c))
}

fn hex_or_hyphen_nohyphen(c: u8) -> bool {
    c.is_ascii_digit() || (b'a'..=b'f').contains(&c) || (b'A'..=b'F').contains(&c)
}

/// v 是否 UUID / ≥32 位无连字符十六进制 / 短摘要（Magpie `idShaped`）。
pub(crate) fn id_shaped(v: &str) -> bool {
    is_uuid(v) || (v.len() >= 32 && !v.contains('-')) || digest_shaped(v)
}

/// 16-31 位单大小写混合字母数字的哈希摘要形状（Magpie `digestShaped`）。
pub(crate) fn digest_shaped(v: &str) -> bool {
    if v.len() < 16 || v.len() >= 32 {
        return false;
    }
    let (mut lower, mut upper, mut digits) = (0usize, 0usize, 0usize);
    for &c in v.as_bytes() {
        match c {
            b'0'..=b'9' => digits += 1,
            b'a'..=b'f' => lower += 1,
            b'A'..=b'F' => upper += 1,
            _ => return false,
        }
    }
    let letters = lower + upper;
    (lower == 0 || upper == 0) && letters >= (v.len() / 8).max(3) && digits >= (v.len() / 8).max(3)
}

/// s[a:b] 所处的 id 整体范围（Magpie `idAround`）。
/// 返回 None 表示不是这种 id 或 s[a:b] 越过其边界。
pub(crate) fn id_around(s: &[u8], a: usize, b: usize) -> Option<(usize, usize)> {
    if element_token(s, a, b) {
        return Some((a, b));
    }
    let (mut l, mut r) = (a, b);
    while l > 0 && hex_or_hyphen(s[l - 1]) {
        l -= 1;
    }
    while r < s.len() && hex_or_hyphen(s[r]) {
        r += 1;
    }
    while l < r && s[l] == b'-' {
        l += 1;
    }
    while r > l && s[r - 1] == b'-' {
        r -= 1;
    }
    if a < l
        || b > r
        || s[l..r].iter().any(|&c| c > 0x7f || !hex_or_hyphen(c))
        || !id_shaped(std::str::from_utf8(&s[l..r]).unwrap_or(""))
    {
        return None;
    }
    // 短摘要自成一词，不是更长十六进制的尾巴（sk_…deadbeef…）
    let mid = std::str::from_utf8(&s[l..r]).unwrap_or("");
    if digest_shaped(mid) && ((l > 0 && word_byte(s, l - 1)) || (r < s.len() && word_byte(s, r))) {
        return None;
    }
    Some((l, r))
}

/// s[l:r] 是否 JSON 对象的键名而非值（Magpie `nameOfField`）。
pub(crate) fn name_of_field(s: &[u8], l: usize, r: usize) -> bool {
    if l == 0 || s[l - 1] != b'"' {
        return false;
    }
    let mut i = r;
    if i < s.len() && s[i] == b'\\' {
        i += 1;
    }
    if i >= s.len() || s[i] != b'"' {
        return false;
    }
    i += 1;
    while i < s.len() && (s[i] == b' ' || s[i] == b'\t') {
        i += 1;
    }
    i < s.len() && s[i] == b':'
}

/// 凭据字段判定（Magpie `credentialField`）。
pub(crate) fn credential_field(s: &[u8], at: usize) -> bool {
    let words = field_words(s, at);
    if words.len() == 1 && words[0] == "key" && key_phrase(s, at) {
        return true;
    }
    if words.len() < 2 && (words.is_empty() || words[0] == "key") || element_field(&words) {
        return false;
    }
    match words[words.len() - 1].as_str() {
        "id" | "ids" | "uuid" | "guid" => return false,
        _ => {}
    }
    for w in &words {
        match w.as_str() {
            "page" | "next" | "cursor" | "continuation" | "csrf" | "xsrf" | "trace" | "request"
            | "idempotency" | "sync" | "nonce" | "hash" | "digest" | "checksum" | "sha"
            | "fingerprint" | "etag" | "cache" | "commit" | "object" | "blob" | "file"
            | "content" | "revision" | "rev" | "partition" | "shard" | "bucket" | "dedup"
            | "dedupe" | "lookup" | "row" | "sort" => return false,
            _ => {}
        }
    }
    true
}

/// 裸 key 是否 "api key: …" 短语的第二词（Magpie `keyPhrase`）。
fn key_phrase(s: &[u8], at: usize) -> bool {
    let from = at.saturating_sub(24);
    let before_full = String::from_utf8_lossy(&s[from..at]).to_lowercase();
    let Some(i) = before_full.rfind("key") else {
        return false;
    };
    let before = before_full[..i].trim_end_matches(' ');
    for w in [
        "api", "access", "secret", "private", "signing", "license", "licence",
    ] {
        if let Some(stripped) = before.strip_suffix(w) {
            if stripped.is_empty() || !word_byte(stripped.as_bytes(), stripped.len() - 1) {
                return true;
            }
        }
    }
    false
}

/// 从 found 里剔除属于 id 的段（Magpie `noIDs`）。
/// 保留 ByWord/ByYours；其余按 id 整体性、kind 与字段名规则剔除。
pub(crate) fn no_ids(s: &[u8], found: Vec<Span>) -> Vec<Span> {
    found
        .into_iter()
        .filter(|f| match f.by {
            FoundBy::Word | FoundBy::Yours => true,
            _ => match id_around(s, f.start, f.end) {
                None => true,
                Some((l, r)) => {
                    if f.start > l || f.end < r {
                        return false; // id 的一片
                    }
                    match f.by {
                        FoundBy::Known => element_token(s, l, r),
                        FoundBy::Model => false, // v1 无本地模型，保守保留
                        FoundBy::Rule => match f.kind {
                            "PASSWORD" | "API_KEY" | "TOKEN" | "PRIVATE_KEY" => false,
                            "SECRET" => {
                                let v = std::str::from_utf8(&s[l..r]).unwrap_or("");
                                if digest_shaped(v) && field_words(s, l).is_empty() {
                                    return false;
                                }
                                !credential_field(s, l)
                            }
                            _ => true,
                        },
                        _ => true,
                    }
                }
            },
        })
        .collect()
}

/// 供测试：uuid 形状。
#[cfg(test)]
pub(crate) fn uuid_check(v: &str) -> bool {
    is_uuid(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex32() -> String {
        // 运行时组装，避免会话管道对敏感形状的转写
        ["d", "e", "a", "d", "b", "e", "e", "f"]
            .iter()
            .cycle()
            .take(32)
            .collect()
    }

    #[test]
    fn uuid_shape() {
        let uuid = ["550e8400-e29b-41d4-a716", "446655440000"].join("-");
        assert!(uuid_check(&uuid));
        assert!(!uuid_check(&hex32()));
    }

    #[test]
    fn digest_shape_rules() {
        // 16 位单大小写、字母数字各占 → 摘要
        let good = ["5d41402abc4b2a7", "6"].concat(); // 16 chars
        assert!(digest_shaped(&good));
        // 全数字不是
        assert!(!digest_shaped(&"1234567890123456".to_string()));
        // 太短
        assert!(!digest_shaped(&"abc123".to_string()));
    }

    #[test]
    fn element_token_shape() {
        let s = b"element_token s0000002a:14 end";
        let a = s.windows(1).rposition(|w| w == b"s").unwrap();
        // s0000002a 在位置 14..23
        let s = b"\"element_token\": \"s0000002a:14\"";
        assert!(element_token(s, 19, 28));
        assert!(!element_token(s, 19, 20));
    }

    #[test]
    fn credential_field_names() {
        let s = b"refresh_token=abcdef";
        assert!(credential_field(s, 14));
        let s = b"next_page_token=abcdef";
        assert!(!credential_field(s, 16));
        let s = b"api key: abcdef";
        assert!(credential_field(s, 9));
    }
}
