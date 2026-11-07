//! 合成密钥形状（Magpie `shape.go`，@f11bf27）。
//!
//! 密钥可以以其同形态的合成键发往上游（Options.Shape）：同前缀、同长度、
//! 同分隔符、同位置同字符类。HMAC 流密码保证同一密钥每次得到同一合成键。
//! 用户名走合成名而非占位符；公网 IP 走文档地址段（RFC 5737/3849）。

use hmac::{Hmac, Mac};
use sha2::Sha256;

use super::super::placeholder::PlaceholderEngine;
use super::{FoundBy, Span};

type HmacSha256 = Hmac<Sha256>;

const MIN_SHAPE: usize = 8;
const MIN_RANDOM: usize = 6;
const STAND_IN_LEN: usize = 8;

/// 密钥前缀（跨多段命名种类），保持原样不合成。
const KEY_PREFIXES: [&str; 7] = [
    "{{APIKEY_wcndvn}}",
    "ghp_",
    "glpat-",
    "sk-proj-",
    "xai-",
    "sk-or-v1-",
    "github_pat_",
];

/// keyHead：v 的开头中命名其种类的部分，保留原样。
pub(crate) fn key_head(v: &str) -> &str {
    for p in KEY_PREFIXES {
        if v.starts_with(p) {
            return p;
        }
    }
    if v.starts_with("AKIA") || v.starts_with("ASIA") {
        return &v[..4];
    }
    let b = v.as_bytes();
    for i in 0..b.len().min(5) {
        let c = b[i];
        if (c == b'-' || c == b'_') && i > 0 {
            return &v[..i + 1];
        }
        if !(c.is_ascii_lowercase() || c.is_ascii_uppercase()) {
            break;
        }
    }
    ""
}

fn shape_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c == b'.'
}

/// shapeOf：v 的合成键（同形、同字符类、HMAC 流密码生成），None 表示不可合成。
pub(crate) fn shape_of(kind: &str, v: &str, engine: &PlaceholderEngine) -> Option<String> {
    if v.len() < MIN_SHAPE || kind == "TERM" {
        return None;
    }
    if !v.bytes().all(shape_byte) {
        return None;
    }
    let head = key_head(v);
    let rest = &v[head.len()..];
    let (mut hex_lower, mut hex_upper) = (true, true);
    for &c in rest.as_bytes() {
        match c {
            b'0'..=b'9' | b'-' | b'_' | b'.' => {}
            b'a'..=b'f' => hex_upper = false,
            b'A'..=b'F' => hex_lower = false,
            _ => {
                hex_lower = false;
                hex_upper = false;
            }
        }
    }
    // HMAC 流密码（与 Go binary.BigEndian.PutUint64 计数器一致）
    let mut mac = HmacSha256::new_from_slice(engine.key_bytes()).ok()?;
    mac.update(b"shape\x00");
    mac.update(kind.as_bytes());
    mac.update(b"\x00");
    mac.update(v.as_bytes());
    let seed = mac.finalize().into_bytes();

    let mut stream: Vec<u8> = Vec::new();
    let mut counter: u64 = 0;
    let mut next = move |k: u8| -> u8 {
        if stream.is_empty() {
            let mut h = HmacSha256::new_from_slice(&seed).unwrap();
            let mut buf = [0u8; 8];
            buf.copy_from_slice(&counter.to_be_bytes());
            counter += 1;
            h.update(&buf);
            stream = h.finalize().into_bytes().to_vec();
        }
        let b = stream.remove(0);
        b % k
    };

    const UPPER: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    const LOWER: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
    const DIGIT: &[u8] = b"0123456789";

    let mut out = head.as_bytes().to_vec();
    let mut made = 0;
    for &c in rest.as_bytes() {
        let set: &[u8] = match c {
            b'-' | b'_' | b'.' => {
                out.push(c);
                continue;
            }
            _ if hex_lower => b"0123456789abcdef",
            _ if hex_upper => b"0123456789ABCDEF",
            _ if c.is_ascii_uppercase() => UPPER,
            _ if c.is_ascii_lowercase() => LOWER,
            _ => DIGIT,
        };
        made += 1;
        let idx = next(set.len() as u8);
        out.push(set[idx as usize]);
    }
    if made < MIN_RANDOM {
        return None;
    }
    let f = String::from_utf8(out).ok()?;
    (&f != v).then_some(f)
}

/// standInOf：用户名的合成名（8 个小写字母，与原值无关）。
pub(crate) fn stand_in_of(v: &str, engine: &PlaceholderEngine) -> String {
    let mut mac = HmacSha256::new_from_slice(engine.key_bytes()).expect("hmac key");
    mac.update(b"user\x00");
    mac.update(v.as_bytes());
    let sum = mac.finalize().into_bytes();
    let mut out = Vec::with_capacity(STAND_IN_LEN);
    for i in 0..STAND_IN_LEN {
        out.push(b'a' + sum[i] % 26);
    }
    String::from_utf8(out).unwrap_or_default()
}

/// IPv4 文档地址段（RFC 5737）。
const IP_NETS: [&str; 3] = ["192.0.2.", "198.51.100.", "203.0.113."];
const IP_HOSTS: usize = 154;

/// ipStandIn：公网 IP 的文档段替身地址。
pub(crate) fn ip_stand_in(v: &str, engine: &PlaceholderEngine) -> Option<String> {
    let addr: std::net::IpAddr = v.parse().ok()?;
    let mut mac = HmacSha256::new_from_slice(engine.key_bytes()).ok()?;
    mac.update(b"ip\x00");
    mac.update(v.as_bytes());
    let sum = mac.finalize().into_bytes();

    match addr {
        std::net::IpAddr::V4(_) => {
            let n = IP_NETS.len() * IP_HOSTS;
            let start = u32::from_be_bytes([sum[0], sum[1], sum[2], sum[3]]) as usize % n;
            Some(format!(
                "{}{}",
                IP_NETS[start / IP_HOSTS],
                101 + start % IP_HOSTS
            ))
        }
        std::net::IpAddr::V6(_) => {
            let mut groups = vec!["2001", "db8"];
            for k in 0..6 {
                let v = u16::from_be_bytes([sum[2 * k], sum[2 * k + 1]]);
                groups.push(format!("{:x}", 0x1000 + (v as u32 % 0xf000)));
            }
            Some(groups.join(":"))
        }
    }
}

/// restoreShapes 的地址还原边界检查（Magpie `addrByte`）。
pub(crate) fn addr_byte(c: u8, v6: bool) -> bool {
    c.is_ascii_alphanumeric() || c == b'.' || (v6 && c == b':')
}

/// shapeTail：流式文本尾部可能是合成键前缀的长度。
pub(crate) fn shape_tail(s: &str, shapes: &[String]) -> usize {
    let mut whole = 0;
    let mut best = 0;
    for f in shapes {
        if let Some(i) = s.rfind(f.as_str()) {
            whole = whole.max(i + f.len());
        }
    }
    for f in shapes {
        let max_k = (f.len() - 1).min(s.len().saturating_sub(whole));
        for k in (best + 1..=max_k).rev() {
            if s.ends_with(&f[..k]) {
                best = k;
                break;
            }
        }
    }
    best
}

/// outsideShapes：从 found 中剔除已在合成键内的段。
pub(crate) fn outside_shapes(s: &str, found: Vec<Span>, shapes: &[String]) -> Vec<Span> {
    let mut found = found;
    for f in shapes {
        let mut i = 0;
        while let Some(j) = s[i..].find(f.as_str()) {
            let a = i + j;
            let z = a + f.len();
            found.retain(|x| x.start >= z || x.end <= a);
            i = z;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aicli::privacy::placeholder::PlaceholderEngine;

    fn engine() -> PlaceholderEngine {
        let mut key = [0u8; 32];
        for (i, b) in key.iter_mut().enumerate() {
            *b = i as u8;
        }
        PlaceholderEngine::with_key(key)
    }

    #[test]
    fn key_head_extraction() {
        assert_eq!(key_head("sk-abcdef12345678901234567890"), "sk-");
        assert_eq!(key_head("ghp_Qw1Ww2Ee3Rr4Tt5Yy6Uu7"), "ghp_");
        assert_eq!(key_head("AKIAIOSFODNN7EXAMPLE"), "AKIA");
        assert_eq!(key_head("plainvalue123"), "");
    }

    #[test]
    fn shape_deterministic() {
        let e = engine();
        let v = ["sk-", "AbCdEfGhIjKlMnOpQrStUvWx"].concat();
        let a = shape_of("API_KEY", &v, &e);
        let b = shape_of("API_KEY", &v, &e);
        assert!(a.is_some());
        assert_eq!(a, b, "same value same shape");
        let shaped = a.unwrap();
        assert!(shaped.starts_with("sk-"), "prefix preserved: {shaped}");
        assert_ne!(shaped, v);
        assert_eq!(shaped.len(), v.len());
    }

    #[test]
    fn shape_too_short_not_shaped() {
        let e = engine();
        assert!(shape_of("API_KEY", "sk-abcd", &e).is_none());
    }

    #[test]
    fn shape_personal_never() {
        let e = engine();
        let v = "abcdefgh12345678";
        assert!(shape_of("TERM", v, &e).is_none());
    }

    #[test]
    fn stand_in_deterministic() {
        let e = engine();
        let a = stand_in_of("alice", &e);
        let b = stand_in_of("alice", &e);
        assert_eq!(a, b);
        assert_eq!(a.len(), 8);
        assert!(a.chars().all(|c| c.is_ascii_lowercase()));
        assert_ne!(a, "alice");
    }

    #[test]
    fn ip_stand_in_shape() {
        let e = engine();
        let f = ip_stand_in("8.8.4.4", &e);
        assert!(f.is_some());
        let addr = f.unwrap();
        assert!(
            addr.starts_with("192.0.2.")
                || addr.starts_with("198.51.100.")
                || addr.starts_with("203.0.113."),
            "doc range: {addr}"
        );
    }
}
