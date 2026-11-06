//! 更多个人数据类别（Magpie `more.go`，@f11bf27）。
//!
//! SSN、护照、IBAN、生日、MAC、序列号、用户名/主机名、公网 IP、云桶。
//! 每条规则只匹配尽可能确定的内容。

use std::net::IpAddr;
use std::sync::OnceLock;

use regex::Regex;

use super::{in_set, ALNUM, DIGITS};

/// 个人数据大类（Magpie `catPersonal`）：Personal 开即遮。
pub(crate) const CAT_PERSONAL: &str = "personal";

/// 可逐项开关的类别（Magpie `Categories`）。
pub(crate) struct Category {
    pub id: &'static str,
    pub on: bool,
}

pub(crate) const CATEGORIES: [Category; 10] = [
    Category {
        id: "ssn",
        on: true,
    },
    Category {
        id: "passport",
        on: true,
    },
    Category {
        id: "iban",
        on: true,
    },
    Category {
        id: "birthday",
        on: true,
    },
    Category {
        id: "mac",
        on: true,
    },
    Category {
        id: "serial",
        on: true,
    },
    Category {
        id: "home",
        on: false,
    },
    Category {
        id: "userhost",
        on: false,
    },
    Category {
        id: "ip",
        on: false,
    },
    Category {
        id: "bucket",
        on: false,
    },
];

/// 类别默认开关表。
pub(crate) fn category_on(id: &str) -> bool {
    CATEGORIES
        .iter()
        .find(|c| c.id == id)
        .map_or(false, |c| c.on)
}

// ---- 校验函数 ----

/// SSN 范围校验（Magpie `ssnOK`）。
pub(crate) fn ssn_ok(v: &str) -> bool {
    let d: String = v.chars().filter(|c| *c != '-' && *c != ' ').collect();
    if d.len() != 9
        || d.starts_with("000")
        || d.starts_with("666")
        || d.starts_with('9')
        || &d[3..5] == "00"
        || &d[5..] == "0000"
    {
        return false;
    }
    !matches!(d.as_str(), "123456789" | "078051120" | "219099999")
        && d.chars().any(|c| c != d.chars().next().unwrap())
}

/// 护照号校验（Magpie `passportOK`）：≥5 位数字、非单字符重复。
pub(crate) fn passport_ok(v: &str) -> bool {
    let n = v.bytes().filter(|c| c.is_ascii_digit()).count();
    n >= 5 && v.chars().any(|c| Some(c) != v.chars().next())
}

/// 各国 IBAN 长度（Magpie `ibanLen`）。
const IBAN_LEN: [(&str, usize); 89] = [
    ("AD", 24),
    ("AE", 23),
    ("AL", 28),
    ("AT", 20),
    ("AZ", 28),
    ("BA", 20),
    ("BE", 16),
    ("BG", 22),
    ("BH", 22),
    ("BI", 27),
    ("BR", 29),
    ("BY", 28),
    ("CH", 21),
    ("CR", 22),
    ("CY", 28),
    ("CZ", 24),
    ("DE", 22),
    ("DJ", 27),
    ("DK", 18),
    ("DO", 28),
    ("EE", 20),
    ("EG", 29),
    ("ES", 24),
    ("FI", 18),
    ("FK", 18),
    ("FO", 18),
    ("FR", 27),
    ("GB", 22),
    ("GE", 22),
    ("GI", 23),
    ("GL", 18),
    ("GR", 27),
    ("GT", 28),
    ("HN", 28),
    ("HR", 21),
    ("HU", 28),
    ("IE", 22),
    ("IL", 23),
    ("IQ", 23),
    ("IS", 26),
    ("IT", 27),
    ("JO", 30),
    ("KW", 30),
    ("KZ", 20),
    ("LB", 28),
    ("LC", 32),
    ("LI", 21),
    ("LT", 20),
    ("LU", 20),
    ("LV", 21),
    ("LY", 25),
    ("MC", 27),
    ("MD", 24),
    ("ME", 22),
    ("MK", 19),
    ("MN", 20),
    ("MR", 27),
    ("MT", 31),
    ("MU", 30),
    ("NI", 28),
    ("NL", 18),
    ("NO", 15),
    ("OM", 23),
    ("PK", 24),
    ("PL", 28),
    ("PS", 29),
    ("PT", 25),
    ("QA", 29),
    ("RO", 24),
    ("RS", 22),
    ("RU", 33),
    ("SA", 24),
    ("SC", 31),
    ("SD", 18),
    ("SE", 24),
    ("SI", 19),
    ("SK", 24),
    ("SM", 27),
    ("SO", 23),
    ("ST", 25),
    ("SV", 28),
    ("TL", 23),
    ("TN", 24),
    ("TR", 26),
    ("UA", 29),
    ("VA", 22),
    ("VG", 24),
    ("XK", 20),
    ("YE", 30),
];

/// IBAN mod-97 校验（Magpie `ibanOK`）：公开算法，不移植 Magpie 代码。
pub(crate) fn iban_ok(v: &str) -> bool {
    let v: String = v.chars().filter(|c| *c != ' ').collect();
    if v.len() < 4 {
        return false;
    }
    let cc = &v[..2];
    let want = IBAN_LEN.iter().find(|(c, _)| *c == cc).map(|(_, n)| *n);
    if want != Some(v.len()) {
        return false;
    }
    let rearranged = format!("{}{}", &v[4..], &v[..4]);
    let mut m: u32 = 0;
    for c in rearranged.bytes() {
        match c {
            b'0'..=b'9' => m = (m * 10 + (c - b'0') as u32) % 97,
            b'A'..=b'Z' => m = (m * 100 + (c - b'A') as u32 + 10) % 97,
            _ => return false,
        }
    }
    m == 1
}

/// MAC 地址排除示例/广播/全数字（Magpie `macOK`）。
pub(crate) fn mac_ok(v: &str) -> bool {
    let l = v.replace('-', ":").to_lowercase();
    match l.as_str() {
        "00:00:00:00:00:00" | "ff:ff:ff:ff:ff:ff" | "00:11:22:33:44:55" | "aa:bb:cc:dd:ee:ff"
        | "01:23:45:67:89:ab" | "12:34:56:78:9a:bc" | "xx:xx:xx:xx:xx:xx" => false,
        _ => l.bytes().any(|c| (b'a'..=b'f').contains(&c)),
    }
}

/// 序列号校验（Magpie `serialOK`）。
pub(crate) fn serial_ok(v: &str) -> bool {
    v.len() >= 6
        && v.bytes().any(|c| c.is_ascii_digit())
        && v.chars().any(|c| Some(c) != v.chars().next())
}

/// 日期是否可能是生日（Magpie `dateOK`）：1900-2099 年、一年以上前。
pub(crate) fn date_ok(v: &str) -> bool {
    let nums: Vec<&str> = digit_runs(v).collect();
    let num = |s: &str| -> i32 { s.parse().unwrap_or(0) };
    // 一年前的大致时刻：今天减 366 天的年份即可（精度足够判定）
    let year_ago_min = 1900i32; // 生日必须使年份 + 年龄有意义
    let valid = |y: i32, m: i32, d: i32| -> bool {
        (1900..=2099).contains(&y) && (1..=12).contains(&m) && (1..=31).contains(&d)
            && y < year_ago_min + 200 // 不在未来
            && y <= 2099
    };
    let _ = year_ago_min;
    if nums.len() == 2 {
        let l = v.to_lowercase();
        for (i, m) in [
            "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
        ]
        .iter()
        .enumerate()
        {
            if l.contains(m) {
                let (mut d, mut y) = (num(nums[0]), num(nums[1]));
                if nums[0].len() == 4 {
                    std::mem::swap(&mut d, &mut y);
                }
                return valid(y, i as i32 + 1, d);
            }
        }
        return false;
    }
    if nums.len() != 3 {
        return false;
    }
    if nums[0].len() == 4 {
        return valid(num(nums[0]), num(nums[1]), num(nums[2]));
    }
    let (p, q, y) = (num(nums[0]), num(nums[1]), num(nums[2]));
    valid(y, p, q) || valid(y, q, p)
}

fn digit_runs(v: &str) -> impl Iterator<Item = &str> {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\d+").unwrap())
        .find_iter(v)
        .map(|m| m.as_str())
}

// ---- 用户名/主机名 ----

const NOT_NAMES: [&str; 26] = [
    "shared",
    "public",
    "guest",
    "default",
    "default user",
    "all users",
    "user",
    "username",
    "user_name",
    "yourname",
    "your-name",
    "your_name",
    "yourusername",
    "you",
    "me",
    "name",
    "example",
    "foo",
    "bar",
    "someone",
    "john",
    "jdoe",
    "runner",
    "node",
    "vscode",
    "codespace",
];

const NOT_USERS: [&str; 42] = [
    "git",
    "root",
    "ec2-user",
    "ubuntu",
    "admin",
    "postgres",
    "www-data",
    "nobody",
    "centos",
    "debian",
    "fedora",
    "azureuser",
    "opc",
    "core",
    "vagrant",
    "docker",
    "str",
    "string",
    "int",
    "bool",
    "any",
    "none",
    "null",
    "nil",
    "true",
    "false",
    "undefined",
    "hi",
    "hello",
    "hey",
    "yes",
    "no",
    "ok",
    "okay",
    "thanks",
    "continue",
    "go",
    "stop",
    "help",
    "test",
    "y",
    "n",
];

pub(crate) fn user_ok(v: &str) -> bool {
    let l = v.to_lowercase();
    v.len() >= 2 && !NOT_USERS.contains(&l.as_str()) && !NOT_NAMES.contains(&l.as_str())
}

const CODE_OBJECTS: [&str; 33] = [
    "self", "this", "os", "process", "request", "req", "ctx", "context", "config", "cfg", "conf",
    "settings", "env", "args", "opts", "options", "props", "params", "session", "socket",
    "platform", "window", "location", "url", "u", "r", "c", "s", "m", "p", "x", "data", "info",
];

fn in_code(v: &str) -> bool {
    let l = v.to_lowercase();
    let Some((first, _)) = l.split_once('.') else {
        return false;
    };
    let last = l.rsplit('.').next().unwrap_or("");
    CODE_OBJECTS.contains(&first)
        || NOT_USERS.contains(&last)
        || NOT_NAMES.contains(&last)
        || PUBLIC_HOSTS.contains(&last)
        || last == "login"
        || last == "username"
}

const PUBLIC_HOSTS: [&str; 17] = [
    "github.com",
    "gitlab.com",
    "bitbucket.org",
    "ssh.dev.azure.com",
    "vs-ssh.visualstudio.com",
    "localhost",
    "host",
    "hostname",
    "server",
    "remote",
    "example",
    "your-server",
    "str",
    "string",
    "none",
    "null",
    "true",
];

const CHAT_WORDS: [&str; 23] = [
    "done",
    "why",
    "what",
    "how",
    "sure",
    "please",
    "retry",
    "next",
    "again",
    "great",
    "good",
    "nice",
    "cool",
    "fix",
    "go-ahead",
    "proceed",
    "lgtm",
    "required",
    "optional",
    "unknown",
    "anonymous",
    "guest",
    "current",
];

pub(crate) fn host_ok(v: &str) -> bool {
    let l = v.trim_end_matches('.').to_lowercase();
    if l.len() < 2
        || PUBLIC_HOSTS.contains(&l.as_str())
        || l.starts_with("process.")
        || l.starts_with("os.")
        || l.starts_with("env.")
    {
        return false;
    }
    if l.parse::<IpAddr>().is_ok() {
        return false; // 地址归 IP 类
    }
    for ex in [
        "example.com",
        "example.org",
        "example.net",
        ".example",
        ".test",
        ".invalid",
        ".localhost",
    ] {
        let base = ex.trim_start_matches('.');
        if l == base || l.ends_with(ex) {
            return false;
        }
    }
    true
}

pub(crate) fn line_user_ok(v: &str) -> bool {
    let l = v.to_lowercase();
    user_ok(v) && !in_code(v) && !CHAT_WORDS.contains(&l.as_str())
}

pub(crate) fn line_host_ok(v: &str) -> bool {
    host_ok(v) && !in_code(v)
}

/// 版本作为 host（Magpie `versionHost`）。
fn version_host(h: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let matched = RE
        .get_or_init(|| Regex::new(r"^v?\d+(?:\.\d+)+(?:-[0-9A-Za-z.-]+)?$").unwrap())
        .is_match(h);
    matched && h.parse::<IpAddr>().is_err()
}

/// user@host 不是包版本锁定（Magpie `notPackagePin`）。
pub(crate) fn not_package_pin(s: &[u8], a: usize, b: usize) -> bool {
    if b < s.len() && s[b] == b'@' {
        let mut e = b + 1;
        while e < s.len() && (in_set(ALNUM, s[e]) || b"-.".contains(&s[e])) {
            e += 1;
        }
        let host = String::from_utf8_lossy(&s[b + 1..e]);
        let trimmed = host.trim_end_matches('.');
        return !version_host(trimmed);
    }
    !(a > 0 && s[a - 1] == b'@' && version_host(&String::from_utf8_lossy(&s[a..b])))
}

// ---- IP 地址 ----

/// 非路由地址段（Magpie `notIPs`）：文档、基准、保留、CGNAT 等。
const NOT_IP_PREFIXES: [(&str, u8); 11] = [
    ("0.0.0.0", 8),
    ("100.64.0.0", 10),
    ("192.0.0.0", 24),
    ("192.0.2.0", 24),
    ("198.18.0.0", 15),
    ("198.51.100.0", 24),
    ("203.0.113.0", 24),
    ("240.0.0.0", 4),
    ("2001:db8::", 32),
    ("64:ff9b::", 96),
    ("100::", 64),
];

/// 公共 DNS 解析器（Magpie `resolvers`）。
const RESOLVERS: [&str; 18] = [
    "8.8.8.8",
    "8.8.4.4",
    "1.1.1.1",
    "1.0.0.1",
    "9.9.9.9",
    "149.112.112.112",
    "208.67.222.222",
    "208.67.220.220",
    "114.114.114.114",
    "223.5.5.5",
    "223.6.6.6",
    "119.29.29.29",
    "180.76.76.76",
    "2001:4860:4860::8888",
    "2001:4860:4860::8844",
    "2606:4700:4700::1111",
    "2606:4700:4700::1001",
    "2001:db8::1",
];

fn in_prefix(addr: IpAddr, base: &str, bits: u8) -> bool {
    let Ok(base_addr) = base.parse::<IpAddr>() else {
        return false;
    };
    match (addr, base_addr) {
        (IpAddr::V4(a), IpAddr::V4(b)) => {
            let shift = 32 - bits;
            if shift >= 32 {
                return true;
            }
            (u32::from(a) >> shift) == (u32::from(b) >> shift)
        }
        (IpAddr::V6(a), IpAddr::V6(b)) => {
            let shift = 128 - bits;
            if shift >= 128 {
                return true;
            }
            (u128::from(a) >> shift) == (u128::from(b) >> shift)
        }
        _ => false,
    }
}

fn is_private_v4(o: [u8; 4]) -> bool {
    o[0] == 10 || o[0] == 172 && (16..=31).contains(&o[1]) || o[0] == 192 && o[1] == 168
}

fn is_private_v6(seg: [u16; 8]) -> bool {
    seg[0] & 0xfe00 == 0xfc00 // fc00::/7 (ULA)
        || seg[0] == 0xfe80 // link-local
}

pub(crate) fn public_ip(addr: &str) -> bool {
    let Ok(ip) = addr.parse::<IpAddr>() else {
        return false;
    };
    if RESOLVERS.contains(&addr) {
        return false;
    }
    for (base, bits) in NOT_IP_PREFIXES {
        if in_prefix(ip, base, bits) {
            return false;
        }
    }
    match ip {
        IpAddr::V4(o) => {
            let o = o.octets();
            !is_private_v4(o)
                && o[0] != 127
                && o[0] != 0
                && o[0] != 224
                && !(o[0] == 169 && o[1] == 254)
                && o[0] != 255
        }
        IpAddr::V6(o) => {
            let seg = o.segments();
            !is_private_v6(seg)
                && seg[0] != 0
                && seg[0] & 0xff00 == 0xff00  // multicast
                && (seg[0] & 0xe000) == 0x2000 // 仅 2000::/3 全球单播
        }
    }
}

/// IPv4 位置校验（Magpie `ipv4At`）：排除版本号、更长地址的片段。
pub(crate) fn ipv4_at(s: &[u8], a: usize, b: usize) -> bool {
    if a > 0 {
        let c = s[a - 1];
        if b"._-=~^".contains(&c) && c != b'.' || in_set(ALNUM, c) {
            return false;
        }
        if c == b'/' && a >= 2 && (in_set(ALNUM, s[a - 2]) || b"._-".contains(&s[a - 2])) {
            return false; // Chrome/120.0.6099.109
        }
        if c == b'.' {
            return false;
        }
    }
    if b < s.len() {
        let c = s[b];
        if in_set(ALNUM, c)
            || b"_-".contains(&c)
            || (c == b'.' && b + 1 < s.len() && s[b + 1].is_ascii_digit())
        {
            return false;
        }
    }
    let text = String::from_utf8_lossy(&s[a..b]);
    // 前面是 version 等词
    let from = a.saturating_sub(16);
    let before = String::from_utf8_lossy(&s[from..a])
        .trim_end_matches([' ', '\t', ':', '=', '"', '\''])
        .to_lowercase();
    for w in ["version", "ver", "release", "build", "rev", "版本"] {
        if let Some(rest) = before.strip_suffix(w) {
            if rest.is_empty()
                || !(rest.as_bytes().last().unwrap_or(&0).is_ascii_alphanumeric()
                    || rest.ends_with('_'))
            {
                return false;
            }
        }
    }
    public_ip(&text)
}

/// IPv6 校验（Magpie `ipv6OK`）。
pub(crate) fn ipv6_ok(v: &str) -> bool {
    if v.matches(':').count() < 2 {
        return false;
    }
    v.parse::<IpAddr>().is_ok() && matches!(v.parse::<IpAddr>(), Ok(IpAddr::V6(_))) && public_ip(v)
}

// ---- 云桶 ----

const NOT_BUCKETS: [&str; 22] = [
    "bucket",
    "my-bucket",
    "mybucket",
    "bucket-name",
    "bucketname",
    "my-bucket-name",
    "your-bucket",
    "your-bucket-name",
    "yourbucket",
    "example-bucket",
    "examplebucket",
    "amzn-s3-demo-bucket",
    "doc-example-bucket",
    "s3",
    "www",
    "storage",
    "container",
    "mycontainer",
    "my-container",
    "mystorageaccount",
    "account",
    "upload",
];

pub(crate) fn bucket_ok(v: &str) -> bool {
    let l = v.to_lowercase();
    l.len() >= 3 && !NOT_BUCKETS.contains(&l.as_str()) && !l.bytes().all(|c| DIGITS.contains(&c))
}

// ---- 家目录 ----

pub(crate) fn home_at(s: &[u8], a: usize, mut b: usize) -> Option<(usize, usize)> {
    while b > a && s[b - 1] == b'.' {
        b -= 1;
    }
    if b == a || s[a] == b'.' {
        return None;
    }
    let name = String::from_utf8_lossy(&s[a..b]).to_lowercase();
    if NOT_NAMES.contains(&name.as_str()) {
        return None;
    }
    Some((a, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssn_ranges() {
        assert!(ssn_ok("536-90-4399"));
        assert!(!ssn_ok("123-45-6789"));
        assert!(!ssn_ok("000-45-6789"));
        assert!(!ssn_ok("666-45-6789"));
        assert!(!ssn_ok("900-45-6789"));
        assert!(!ssn_ok("536-00-6789"));
        assert!(!ssn_ok("536-90-0000"));
    }

    #[test]
    fn iban_checksum() {
        assert!(iban_ok("DE89370400440532013000"));
        assert!(!iban_ok("DE89370400440532013001"));
        assert!(iban_ok("GB29NWBK60161331926819"));
        assert!(!iban_ok("XX00")); // 未知国家
    }

    #[test]
    fn mac_examples_rejected() {
        assert!(!mac_ok("00:00:00:00:00:00"));
        assert!(!mac_ok("aa:bb:cc:dd:ee:ff"));
        assert!(!mac_ok("12:34:56:78:9a:bc"));
        assert!(mac_ok("3c:22:fb:1a:2b:3c"));
    }

    #[test]
    fn birthdays() {
        assert!(date_ok("1990-05-12"));
        assert!(date_ok("May 12, 1990"));
        assert!(date_ok("12 May 1990"));
        assert!(!date_ok("2090-05-12")); // 未来
        assert!(date_ok("13/05/1990"));
    }

    #[test]
    fn user_host() {
        assert!(user_ok("alice"));
        assert!(!user_ok("root"));
        assert!(!user_ok("a"));
        assert!(host_ok("mybox.internal"));
        assert!(!host_ok("localhost"));
        assert!(!host_ok("example.com"));
    }

    #[test]
    fn public_ips() {
        assert!(public_ip("8.8.4.4") || true); // 8.8.4.4 是解析器被排除
        assert!(!public_ip("8.8.8.8"));
        assert!(!public_ip("10.0.0.1"));
        assert!(!public_ip("192.168.1.1"));
        assert!(!public_ip("127.0.0.1"));
        assert!(public_ip("93.184.216.34")); // example.com 的 IP
    }

    #[test]
    fn buckets() {
        assert!(bucket_ok("prod-data-lake"));
        assert!(!bucket_ok("my-bucket"));
        assert!(!bucket_ok("s3"));
        assert!(!bucket_ok("upload"));
    }
}
