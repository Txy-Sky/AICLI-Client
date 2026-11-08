//! 检测规则引擎主体（Magpie `redact.go` 的规则表与 mask/scan 引擎，@f11bf27）。
//!
//! 这是 E-03 的核心：`Rule` 定义 + ~60 条正则规则 + `Options` + `mask` 引擎。
//! 逐条移植 Magpie 的规则，语义一致。

use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;

use super::super::placeholder::PlaceholderEngine;
use super::dateword;
use super::fieldvalue;
use super::fieldvalue::not_a_slug;
use super::ids;
use super::known;
use super::more;
use super::phone;
use super::{FoundBy, Span};

// ---- 字符集 ----

const ALNUM: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
const TOKEN_CH: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";
const DIGITS: &[u8] = b"0123456789";
const QUOTE: &str = r#"(?:\\?["'])?"#;

// ---- Rule 结构 ----

/// fix 函数签名。
pub(crate) type FixFn = fn(&str, usize, usize) -> Option<(usize, usize)>;

/// 一条检测规则（Magpie `rule`）。
#[derive(Clone)]
pub(crate) struct Rule {
    pub kind: &'static str,
    pub pattern: &'static str,
    /// 触发词：文本中含其中之一才运行正则（性能优化）。
    pub markers: &'static [&'static str],
    /// 折叠触发词（小写化后匹配）。
    pub fold: &'static [&'static str],
    /// 边界字符：匹配两侧不得接触。
    pub bound: &'static str,
    /// 进一步校验。
    pub ok: Option<fn(&str) -> bool>,
    /// 位置校验。
    pub at: Option<fn(&str, usize, usize) -> bool>,
    /// 修复匹配范围。
    pub fix: Option<FixFn>,
    /// 折叠回看/前看距离。
    pub back: usize,
    pub ahead: usize,
    /// 类别（空 = 密钥类）。
    pub cat: &'static str,
    /// 用户自定义。
    pub yours: bool,
    /// 任一捕获组（多分支各有组）。
    pub any_group: bool,
    /// 组类型（分别遮盖）。
    pub groups: &'static [&'static str],
    /// 各组的 ok 校验（Magpie `groupOK`）。
    pub group_ok: &'static [fn(&str) -> bool],
}

impl Rule {
    /// 默认值（规则字面量 struct update 语法用；未写字段取此，对齐 Go 零值）。
    const fn base() -> Rule {
        Rule {
            kind: "",
            pattern: "",
            markers: &[],
            fold: &[],
            bound: "",
            ok: None,
            at: None,
            fix: None,
            back: 0,
            ahead: 0,
            cat: "",
            yours: false,
            any_group: false,
            groups: &[],
            group_ok: &[],
        }
    }

    /// 编译正则（懒加载）。
    /// 编译并缓存（leak 后全局存活，每条规则仅编译一次）。
    fn re(&self) -> &'static Regex {
        static CACHE: std::sync::Mutex<Option<HashMap<&'static str, &'static Regex>>> =
            std::sync::Mutex::new(None);
        let mut guard = CACHE.lock().unwrap_or_else(|p| p.into_inner());
        let cache = guard.get_or_insert_with(HashMap::new);
        if let Some(re) = cache.get(self.pattern) {
            return re;
        }
        let re: &'static Regex = Box::leak(
            Regex::new(self.pattern)
                .expect("rule regex compiles")
                .into(),
        );
        cache.insert(self.pattern, re);
        re
    }

    /// 哪个发现器（Magpie `rule.by()`）。
    fn by(&self) -> FoundBy {
        if self.yours {
            FoundBy::Yours
        } else {
            FoundBy::Rule
        }
    }

    /// 在 s 中匹配（Magpie `rule.matches`）：ahead 为 0 时全串扫描；
    /// 否则只在小写文本 lower 中各 fold 标记前后 back/ahead 的窗口内运行
    /// （大小写不敏感词的模式对长文本慢，Magpie 只在标记附近跑）。
    fn matches(&self, s: &str, lower: &str) -> Vec<Vec<Option<(usize, usize)>>> {
        let re = self.re();
        let mut out = Vec::new();
        fn push(caps: regex::Captures<'_>, off: usize, out: &mut Vec<Vec<Option<(usize, usize)>>>) {
            let mut groups = Vec::with_capacity(caps.len());
            for i in 0..caps.len() {
                groups.push(caps.get(i).map(|m| (m.start() + off, m.end() + off)));
            }
            out.push(groups);
        }
        if self.ahead == 0 {
            for caps in re.captures_iter(s) {
                push(caps, 0, &mut out);
            }
            return out;
        }
        // fold 窗口（Magpie rule.matches）：从行首或不早于 back 的词首，
        // 到标记后 ahead 处；相邻窗口合并
        let bytes = s.as_bytes();
        let mut wins: Vec<(usize, usize)> = Vec::new();
        for f in self.fold {
            let mut i = 0;
            while let Some(j) = lower[i..].find(f) {
                let at = i + j;
                i = at + f.len();
                let mut start = at.saturating_sub(self.back);
                // 按字节找换行：start 可能落在多字节字符中间，切 &str 会 panic
                //（Go 按字节切串不会，这是移植差异）。
                if let Some(nl) = bytes[start..at].iter().rposition(|&c| c == b'\n') {
                    start += nl + 1;
                } else if start > 0 {
                    while start < at
                        && (TOKEN_STR.as_bytes().contains(&bytes[start - 1])
                            || bytes[start - 1] == b'.'
                            || bytes[start] & 0xc0 == 0x80)
                    {
                        start += 1; // 回到词首 / 多字节字符边界
                    }
                    if start == at
                        && (ALNUM_STR.as_bytes().contains(&bytes[at - 1]) || bytes[at - 1] == b'_')
                    {
                        continue; // 标记在更长单词内部
                    }
                }
                // 窗口终点向前收到字符边界：Go 的 s[a:b] 可切在字符中间，Rust 不行。
                let mut end = (i + self.ahead).min(s.len());
                while !s.is_char_boundary(end) {
                    end -= 1;
                }
                wins.push((start, end));
            }
        }
        wins.sort_by_key(|w| w.0);
        let mut k = 0;
        while k < wins.len() {
            let mut w = wins[k];
            while k + 1 < wins.len() && wins[k + 1].0 <= w.1 {
                k += 1;
                w.1 = w.1.max(wins[k].1);
            }
            for caps in re.captures_iter(&s[w.0..w.1]) {
                push(caps, w.0, &mut out);
            }
            k += 1;
        }
        out
    }

    /// 匹配是否可接受（Magpie `rule.takes`）：范围有效 + 边界 + ok + at。
    /// ok 只用调用方传入的（组规则传 groupOK[g]，普通规则传 r.ok）。
    fn takes(&self, s: &str, a: usize, b: usize, ok: Option<fn(&str) -> bool>) -> bool {
        if b <= a {
            return false;
        }
        if !self.bound.is_empty() {
            let bytes = s.as_bytes();
            if a > 0 && self.bound.as_bytes().contains(&bytes[a - 1]) {
                return false;
            }
            if b < bytes.len() && self.bound.as_bytes().contains(&bytes[b]) {
                return false;
            }
        }
        if let Some(check) = ok {
            if !check(&s[a..b]) {
                return false;
            }
        }
        if let Some(position_check) = self.at {
            if !position_check(s, a, b) {
                return false;
            }
        }
        true
    }
}

// ---- 辅助校验 ----

/// notAVariable（Magpie）。
pub(crate) fn not_a_variable(v: &str) -> bool {
    if let Some(first) = v.chars().next() {
        if "$%{<*".contains(first) {
            return false;
        }
    }
    !v.trim_matches(['*', 'x', 'X', '.']).is_empty()
}

/// keyValue（Magpie）。
pub(crate) fn key_value(v: &str) -> bool {
    if !not_a_variable(v) {
        return false;
    }
    let has_letter = v.chars().any(|r| r.is_ascii_alphabetic());
    has_letter && (v.bytes().any(|c| c.is_ascii_digit()) || v.len() >= 16)
}

const KEY_SEPARATORS: [&str; 8] = ["为", "為", "是", "：", ":", "=", "は", "는"];
const QUANTITY_UNITS: [&str; 26] = [
    "位",
    "比特",
    "字节",
    "字節",
    "个",
    "個",
    "次",
    "秒",
    "毫秒",
    "分钟",
    "分鐘",
    "小时",
    "小時",
    "天",
    "年",
    "bit",
    "byte",
    "ms",
    "sec",
    "min",
    "ビット",
    "バイト",
    "비트",
    "바이트",
    "",
    "",
];

/// keyAt（Magpie）。
pub(crate) fn key_at(s: &str, a: usize, b: usize) -> bool {
    let v = &s[a..b];
    if fieldvalue::path_value(v) || fieldvalue::version_value(v) {
        return false;
    }
    if key_value(v) {
        return true;
    }
    if v.len() < 6 || !v.bytes().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let before = s[..a].trim_end_matches([' ', '\t', '"', '\'', '“', '「', '\\']);
    let after = s[b..].trim_start_matches([' ', '\t']);
    KEY_SEPARATORS.iter().any(|c| before.ends_with(c))
        && !QUANTITY_UNITS
            .iter()
            .filter(|u| !u.is_empty())
            .any(|u| after.starts_with(u))
}

/// secretValue（Magpie）。
pub(crate) fn secret_value(v: &str) -> bool {
    if !not_a_variable(v) || v.starts_with("process.env") || v.starts_with("os.") {
        return false;
    }
    v.bytes().any(|c| c.is_ascii_digit()) && v.chars().any(|r| r.is_ascii_alphabetic())
}

/// notCode（Magpie）。
pub(crate) fn not_code(s: &str, a: usize, b: usize) -> bool {
    static IDENT_PATH: OnceLock<Regex> = OnceLock::new();
    let re = IDENT_PATH
        .get_or_init(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z0-9_]+)*$").unwrap());
    if !re.is_match(&s[a..b]) {
        return true;
    }
    let rest = s[b..].trim_start_matches([' ', '\t']);
    if rest.starts_with('(') {
        return false;
    }
    let bytes = s.as_bytes();
    if a > 0 && (bytes[a - 1] == b'"' || bytes[a - 1] == b'\'')
        || !s[..a].trim_end_matches([' ', '\t']).ends_with(':')
    {
        return true;
    }
    rest.is_empty() || !";,)}]|?&<>".contains(rest.chars().next().unwrap())
}

/// notUserinfo（Magpie）。
pub(crate) fn not_userinfo(s: &str, a: usize, _b: usize) -> bool {
    !in_userinfo(s, a)
}

fn in_userinfo(s: &str, a: usize) -> bool {
    let bytes = s.as_bytes();
    let mut i = a;
    while i > 0 && !b" \t\r\n/?#\"'<>@`".contains(&bytes[i - 1]) {
        i -= 1;
    }
    if i < 3 || bytes[i - 1] != b'/' || bytes[i - 2] != b'/' || bytes[i - 3] != b':' {
        return false;
    }
    let j = i - 3;
    let mut k = j;
    while k > 0 {
        let c = bytes[k - 1];
        if c.is_ascii_alphanumeric() || b"+.-".contains(&c) {
            k -= 1;
        } else {
            break;
        }
    }
    while k < j && !bytes[k].is_ascii_alphabetic() {
        k += 1;
    }
    k < j
}

/// emailAt（Magpie）。
pub(crate) fn email_at(s: &str, a: usize, b: usize) -> bool {
    let bytes = s.as_bytes();
    if a > 0 && bytes[a - 1] == b'.' {
        return false;
    }
    if b + 1 < bytes.len() && bytes[b] == b'.' && TOKEN_CH.contains(&bytes[b + 1]) {
        return false;
    }
    not_userinfo(s, a, b)
}

/// hiddenInURL（Magpie）。
pub(crate) fn hidden_in_url(s: &str, a: usize, b: usize) -> bool {
    let pw = &s[a..b];
    let Some(i) = s[..a].rfind("//") else {
        return true;
    };
    if s.as_bytes().get(a - 1) != Some(&b':') {
        return true;
    }
    if &s[i + 2..a - 1] == pw {
        return false;
    }
    let bytes = s.as_bytes();
    let mut end = b + 1;
    while end < bytes.len() && !b" \t\r\n\"'<>`\\)]}".contains(&bytes[end]) {
        end += 1;
    }
    if b + 1 >= end {
        return true;
    }
    let tail = &s[b + 1..end];
    for part in tail.split(|c: char| "/:.?#&=;,@".contains(c)) {
        if part == pw {
            return false;
        }
    }
    true
}

/// realEmail（Magpie）。
pub(crate) fn real_email(v: &str) -> bool {
    let Some(at) = v.rfind('@') else { return false };
    let local = &v[..at];
    if local.eq_ignore_ascii_case("git") {
        return false;
    }
    let host = v[at + 1..].to_lowercase();
    if !mail_host(local, &host) {
        return false;
    }
    if file_host(&host) || DOC_FILES.contains(&host.as_str()) || !mail_tld(&host) {
        return false;
    }
    for fake in [
        "example.com",
        "example.org",
        "example.net",
        "localhost",
        ".test",
        ".invalid",
        ".example",
    ] {
        let base = fake.trim_start_matches('.');
        if host == base || host.ends_with(fake) {
            return false;
        }
    }
    !host.starts_with("noreply") && !v.contains("noreply@")
}

const LINKER_WORDS: [&str; 5] = ["plt", "got", "sec", "iplt", "igot"];
const DOC_FILES: [&str; 15] = [
    "agents.md",
    "claude.md",
    "gemini.md",
    "readme.md",
    "skill.md",
    "changelog.md",
    "contributing.md",
    "license.md",
    "lessons.md",
    "todo.md",
    "copilot-instructions.md",
    "crush.md",
    "qwen.md",
    "iflow.md",
    "warp.md",
];
const LOCAL_TLDS: [&str; 7] = [
    "local",
    "internal",
    "lan",
    "corp",
    "home",
    "intranet",
    "localdomain",
];

const FILE_EXT: [&str; 69] = [
    "jpg", "jpeg", "png", "gif", "webp", "svg", "heic", "heif", "avif", "bmp", "tif", "tiff",
    "ico", "icns", "pdf", "svgz", "apng", "jxl", "psd", "eps", "lottie", "mp4", "m4v", "webm",
    "mkv", "avi", "mp3", "m4a", "wav", "ogg", "flac", "aac", "ttf", "otf", "woff", "woff2", "eot",
    "json", "yaml", "yml", "toml", "xml", "txt", "csv", "tsv", "log", "html", "htm", "css", "scss",
    "less", "js", "mjs", "cjs", "jsx", "ts", "tsx", "doc", "docx", "xls", "xlsx", "ppt", "pptx",
    "gz", "tgz", "tar", "rar", "7z", "dmg",
];

const SCALE_EXT: [&str; 4] = ["zip", "mov", "md", "map"];

fn mail_host(local: &str, host: &str) -> bool {
    if local.ends_with('@') || !host.contains('.') || host.trim_matches('.') != host {
        return false;
    }
    let mut linker = true;
    for label in host.split('.') {
        if label.is_empty()
            || label
                .chars()
                .any(|r| r.is_ascii() && r != '-' && !r.is_ascii_lowercase() && !r.is_ascii_digit())
        {
            return false;
        }
        if !LINKER_WORDS.contains(&label) {
            linker = false;
        }
    }
    !linker
}

fn mail_tld(host: &str) -> bool {
    let tld = host.rsplit('.').next().unwrap_or("");
    if LOCAL_TLDS.contains(&tld) {
        return true;
    }
    // v1 近似：不在文件扩展名列表中的 TLD 即可（完整 PSL 检查需 publicsuffix crate）
    !FILE_EXT.contains(&tld)
}

const COMMON_TLDS: [&str; 30] = [
    "com", "org", "net", "edu", "gov", "mil", "int", "io", "co", "uk", "de", "fr", "jp", "cn",
    "kr", "in", "au", "ca", "br", "ru", "it", "es", "nl", "se", "no", "fi", "dk", "at", "ch", "be",
];

fn file_host(host: &str) -> bool {
    let Some(dot) = host.rfind('.') else {
        return false;
    };
    let ext = &host[dot + 1..];
    FILE_EXT.contains(&ext)
        || (dot > 0
            && SCALE_EXT.contains(&ext)
            && Regex::new(r"^[0-9]+(?:\.[0-9]+)?x$")
                .unwrap()
                .is_match(&host[..dot]))
}

/// chineseID（Magpie）。
pub(crate) fn chinese_id(v: &str) -> bool {
    const W: [i32; 17] = [7, 9, 10, 5, 8, 4, 2, 1, 6, 3, 7, 9, 10, 5, 8, 4, 2];
    let bytes = v.as_bytes();
    let mut sum = 0i32;
    for i in 0..17 {
        sum += (bytes[i] - b'0') as i32 * W[i];
    }
    let check = "10X98765432".as_bytes()[(sum % 11) as usize];
    v[17..].to_uppercase().as_bytes()[0] == check
}

/// luhn（Magpie）。
pub(crate) fn luhn(v: &str) -> bool {
    let ds: Vec<u8> = v
        .bytes()
        .filter(|c| c.is_ascii_digit())
        .map(|c| c - b'0')
        .collect();
    if ds.len() < 15 || ds.len() > 19 {
        return false;
    }
    let mut sum = 0u32;
    for i in 0..ds.len() {
        let mut d = ds[ds.len() - 1 - i] as u32;
        if i % 2 == 1 {
            d *= 2;
            if d > 9 {
                d -= 9;
            }
        }
        sum += d;
    }
    sum % 10 == 0
}

// ---- 规则表（Magpie redact.go `rules` + more.go `moreRules`，逐条对应）----

/// bound 字符集的字符串形式。
const ALNUM_STR: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
const TOKEN_STR: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";
const DIGITS_STR: &str = "0123456789";
const BOUND_ALNUM_HYPHEN: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-";
const BOUND_EMAIL: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_%+-";
const BOUND_MAC: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789:-";
const BOUND_IP6: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789:_";

/// 密钥字段名的 markers（Magpie fieldNames.markers()：小写/大写/首字母大写）。
const SECRET_MARKERS: &[&str] = &[
    "pass",
    "PASS",
    "Pass",
    "secret",
    "SECRET",
    "Secret",
    "token",
    "TOKEN",
    "Token",
    "key",
    "KEY",
    "Key",
    "credential",
    "CREDENTIAL",
    "Credential",
];

/// cjkPasswordNames（Magpie）：CJK 里对密码的称呼。
const CJK_PASSWORD_NAMES: &[&str] = &[
    "密码",
    "密碼",
    "口令",
    "PIN码",
    "PIN碼",
    "PIN 码",
    "安全码",
    "安全碼",
    "パスワード",
    "暗証番号",
    "비밀번호",
    "암호",
    "비번",
];
/// cjkKeyNames（Magpie）：CJK 里对密钥/令牌的称呼。
const CJK_KEY_NAMES: &[&str] = &[
    "密钥",
    "密鑰",
    "秘钥",
    "秘鑰",
    "私钥",
    "私鑰",
    "金钥",
    "金鑰",
    "令牌",
    "权杖",
    "權杖",
    "授权码",
    "授權碼",
    "秘密鍵",
    "トークン",
    "シークレット",
    "APIキー",
    "アクセスキー",
    "토큰",
    "비밀키",
    "API 키",
    "API키",
];
/// cjkCopulas（Magpie）：中文消息里名字与值之间的词。
const CJK_COPULAS: &[&str] = &["为", "為", "是", "："];
/// bucketHosts（Magpie more.go）：云存储的主机名（fold 标记）。
const BUCKET_HOSTS: &[&str] = &[
    "amazonaws.com",
    "googleapis.com",
    "cloud.google.com",
    "aliyuncs.com",
    "myqcloud.com",
    "windows.net",
    "cloudflarestorage.com",
    "digitaloceanspaces.com",
    "myhuaweicloud.com",
    "volces.com",
];

/// lowerASCII（Magpie）：仅小写化 ASCII 字母，字节长度不变，
/// 与原文位置一一对应。
fn lower_ascii(s: &str) -> String {
    let b: Vec<u8> = s
        .bytes()
        .map(|c| if c.is_ascii_uppercase() { c + 32 } else { c })
        .collect();
    String::from_utf8(b).expect("lowerASCII preserves utf-8")
}

// ---- more.go 规则的 at/fix 适配（more.rs 是字节版签名）----

fn at_not_package_pin(s: &str, a: usize, b: usize) -> bool {
    more::not_package_pin(s.as_bytes(), a, b)
}

fn at_ipv4(s: &str, a: usize, b: usize) -> bool {
    more::ipv4_at(s.as_bytes(), a, b)
}

fn fix_home_at(s: &str, a: usize, b: usize) -> Option<(usize, usize)> {
    more::home_at(s.as_bytes(), a, b)
}

/// ibanAt（Magpie more.go）：s[a:b] 是 IBAN；不是时去掉最后一个空格后的
/// 词再试（BE68 5390 0754 7034 THEN 里被当成末组的 THEN）。
fn fix_iban_at(s: &str, a: usize, mut b: usize) -> Option<(usize, usize)> {
    loop {
        if more::iban_ok(&s[a..b]) {
            return Some((a, b));
        }
        let i = s[a..b].rfind(' ')?;
        b = a + i;
    }
}

/// 核心规则集（Magpie redact.go `rules`，尾部接 more.go `moreRules`，顺序一致）。
fn core_rules() -> Vec<Rule> {
    vec![
        // ===== redact.go 主表 =====
        Rule {
            kind: "PRIVATE_KEY",
            pattern: r"-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY(?: BLOCK)?-----[\s\S]+?-----END (?:[A-Z0-9]+ )*PRIVATE KEY(?: BLOCK)?-----",
            markers: &["PRIVATE KEY"],
            ..Rule::base()
        },
        Rule {
            kind: "API_KEY",
            pattern: r"sk-(?:ant-|proj-|or-|svcacct-|admin-)?[A-Za-z0-9_-]{20,}",
            markers: &["sk-"],
            bound: TOKEN_STR,
            ok: Some(not_a_slug),
            ..Rule::base()
        },
        // 网关/中继自己的键：oc_sk_…、or_sk_…（#195）
        Rule {
            kind: "API_KEY",
            pattern: r"[a-z]{2,4}_sk_[A-Za-z0-9_-]{20,}",
            markers: &["_sk_"],
            bound: TOKEN_STR,
            ok: Some(secret_value),
            ..Rule::base()
        },
        Rule {
            kind: "API_KEY",
            pattern: r"(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,}|glpat-[A-Za-z0-9_-]{20,})",
            markers: &["gh", "github_pat_", "glpat-"],
            bound: TOKEN_STR,
            ..Rule::base()
        },
        Rule {
            kind: "API_KEY",
            pattern: r"AIza[0-9A-Za-z_-]{35}",
            markers: &["AIza"],
            bound: TOKEN_STR,
            ..Rule::base()
        },
        Rule {
            kind: "API_KEY",
            pattern: r"xox[abposr]-[0-9A-Za-z-]{10,}",
            markers: &["xox"],
            bound: TOKEN_STR,
            ..Rule::base()
        },
        Rule {
            kind: "API_KEY",
            pattern: r"(?:sk|rk)_live_[0-9A-Za-z]{20,}",
            markers: &["_live_"],
            bound: TOKEN_STR,
            ..Rule::base()
        },
        Rule {
            kind: "API_KEY",
            pattern: r"(?:AKIA|ASIA)[0-9A-Z]{16}",
            markers: &["AKIA", "ASIA"],
            bound: ALNUM_STR,
            ..Rule::base()
        },
        Rule {
            kind: "API_KEY",
            pattern: r"(?:hf_[A-Za-z0-9]{30,}|gsk_[A-Za-z0-9]{40,}|xai-[A-Za-z0-9]{40,}|npm_[A-Za-z0-9]{36}|pypi-[A-Za-z0-9_-]{50,}|dop_v1_[a-f0-9]{64}|SG\.[A-Za-z0-9_-]{22}\.[A-Za-z0-9_-]{43})",
            markers: &["hf_", "gsk_", "xai-", "npm_", "pypi-", "dop_v1_", "SG."],
            bound: TOKEN_STR,
            ..Rule::base()
        },
        Rule {
            kind: "TOKEN",
            pattern: r"eyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}",
            markers: &["eyJ"],
            bound: TOKEN_STR,
            ..Rule::base()
        },
        // user:password@host 的密码（含 :password@ 无用户的形式）
        Rule {
            kind: "PASSWORD",
            pattern: r#"[A-Za-z][A-Za-z0-9+.-]*://[^\s:@/?#"'<>]*:([^\s@/?#"'<>]{3,})@"#,
            markers: &["://"],
            ok: Some(not_a_variable),
            at: Some(hidden_in_url),
            ..Rule::base()
        },
        // password = …、API_KEY: "…"（.env/配置文件）
        Rule {
            kind: "SECRET",
            pattern: r#"(?i)[A-Za-z0-9_.-]*(?:password|passwd|secret|token|api[_-]?key|access[_-]?key|private[_-]?key|credential)[A-Za-z0-9_.-]*(?:\\?["'])?[ \t]*[:=][ \t]*(?:\\?["'])?([A-Za-z0-9_\-./+=~!@#%^&*]{8,})"#,
            markers: SECRET_MARKERS,
            ok: Some(secret_value),
            at: Some(fieldvalue::field_at),
            ..Rule::base()
        },
        // 只叫 key 的字段（providers.json 的 "key": "oc_sk_…"，#195）：
        // 值须 16+ 字符且字母数字都有（map 的 "key": "value" 不是）
        Rule {
            kind: "SECRET",
            pattern: r#"(?i)(?:^|[^A-Za-z0-9_.-])(?:\\?["'])?key(?:\\?["'])?[ \t]*[:=][ \t]*(?:\\?["'])?([A-Za-z0-9_\-./+=~!@#%^&*]{16,})"#,
            markers: &["key", "KEY", "Key"],
            ok: Some(secret_value),
            at: Some(fieldvalue::field_at),
            ..Rule::base()
        },
        // CJK 密码（Magpie cjkNamed(cjkPasswordNames)）：密码：…、密碼是 …。
        // 代码里没有叫 密码 的字段，四个字符以上的值都取，纯数字也取
        Rule {
            kind: "PASSWORD",
            pattern: r#"(?i)(?:密码|密碼|口令|PIN码|PIN碼|PIN 码|安全码|安全碼|パスワード|暗証番号|비밀번호|암호|비번)[ \t]*(?:[:：=][ \t]*|[为為是][ \t]*[:：]?[ \t]*|は[ \t]*|는[ \t]*)?(?:\\?["'“「])?([A-Za-z0-9_\-./+=~!@#%^&*]{3,63}[A-Za-z0-9_\-/+=~!@#%^&*])"#,
            fold: CJK_PASSWORD_NAMES,
            back: 64,
            ahead: 100,
            ok: Some(not_a_variable),
            ..Rule::base()
        },
        // CJK 密钥/令牌（Magpie cjkNamed(cjkKeyNames)）：密钥为 …、令牌是 …。
        // 纯数字的键在 为/是/：/= 后且 6 位以上、不计数时也取
        Rule {
            kind: "SECRET",
            pattern: r#"(?i)(?:密钥|密鑰|秘钥|秘鑰|私钥|私鑰|金钥|金鑰|令牌|权杖|權杖|授权码|授權碼|秘密鍵|トークン|シークレット|APIキー|アクセスキー|토큰|비밀키|API 키|API키)[ \t]*(?:[:：=][ \t]*|[为為是][ \t]*[:：]?[ \t]*|は[ \t]*|는[ \t]*)?(?:\\?["'“「])?([A-Za-z0-9_\-./+=~!@#%^&*]{3,63}[A-Za-z0-9_\-/+=~!@#%^&*])"#,
            fold: CJK_KEY_NAMES,
            back: 64,
            ahead: 100,
            at: Some(key_at),
            ..Rule::base()
        },
        // 英文名 + CJK 系词（apikey 为 …、token 是 …、API Key：…）
        Rule {
            kind: "SECRET",
            pattern: r#"(?i)(?:password|passwd|secret|token|api[_-]?key|access[_-]?key|private[_-]?key|credential|pwd|api[ ]key|access[ ]key|secret[ ]key|key|\bpin(?:[ ]?code)?)(?:\\?["'])?[ \t]*(?:[为為是][ \t]*[:：]?|：)[ \t]*(?:\\?["'“「])?([A-Za-z0-9_\-./+=~!@#%^&*]{3,}[A-Za-z0-9_\-/+=~!@#%^&*])"#,
            fold: CJK_COPULAS,
            back: 24,
            ahead: 260,
            at: Some(key_at),
            ..Rule::base()
        },
        // ===== 我方新增（E-07 Maskit 对照发现的漏遮，非 Magpie 移植）=====
        // 连接串口令含 @ 或 #：Magpie 的口令字符类排除二者（URL 语法），
        // postgres://user:P@ssw0rd@host:5432/db 整条不遮。锚定最后一个 @
        // 之后的 host[:port]，口令段（组 1）须含 @/# 才触发（@/# 可在首位
        // 之后任意位置，用户名可空：redis://:P@ss@host）；不含时由上面的
        // Magpie 规则处理（span 相同，去重取更长）。
        Rule {
            kind: "PASSWORD",
            pattern: r#"(?i)\b[a-z][a-z0-9+.-]*://[^\s:@/?#"<>]{0,64}:([^\s/?#"<>]*?[@#][^\s/?#"<>]*?)@[a-z0-9][a-z0-9.-]{2,}(?::[0-9]{2,5})?(?:[/\s"'?,;)\]>]|$)"#,
            markers: &["://"],
            ok: Some(not_a_variable),
            at: Some(hidden_in_url),
            ..Rule::base()
        },
        // 阿里云 AccessKey ID（LTAI + 16~24 位字母数字；官方格式）
        Rule {
            kind: "API_KEY",
            pattern: r"LTAI[0-9A-Za-z]{16,24}",
            markers: &["LTAI"],
            bound: ALNUM_STR,
            ok: Some(secret_value),
            ..Rule::base()
        },
        // AccessKeySecret 赋值（SK：/SK=/access_key_secret=，24+ 位字母数字）。
        // 名字前须是非字母数字（TASK:/DISK=/RISK: 不算 SK；AWS_SK= 算）。
        Rule {
            kind: "SECRET",
            pattern: r#"(?i)(?:^|[^a-z0-9])(?:SK|access[_-]?key[_-]?secret)[ 	]*[：:=][ 	]*["']?([A-Za-z0-9]{24,})"#,
            markers: &["SK", "sk", "Sk", "SECRET", "secret", "Secret"],
            ok: Some(secret_value),
            at: Some(fieldvalue::field_at),
            ..Rule::base()
        },
        // ===== 以下为 catPersonal（personal 开关）=====
        Rule {
            kind: "EMAIL",
            pattern: r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}",
            markers: &["@"],
            bound: BOUND_EMAIL,
            ok: Some(real_email),
            at: Some(email_at),
            cat: more::CAT_PERSONAL,
            ..Rule::base()
        },
        Rule {
            kind: "ID_CARD",
            pattern: r"[1-9]\d{5}(?:18|19|20)\d{2}(?:0[1-9]|1[0-2])(?:0[1-9]|[12]\d|3[01])\d{3}[\dXx]",
            bound: ALNUM_STR,
            ok: Some(chinese_id),
            cat: more::CAT_PERSONAL,
            ..Rule::base()
        },
        Rule {
            kind: "PHONE",
            pattern: r"(?:\+?86[- ]?)?1[3-9]\d{9}",
            bound: ALNUM_STR,
            at: Some(phone::phone_at),
            cat: more::CAT_PERSONAL,
            ..Rule::base()
        },
        Rule {
            kind: "BANK_CARD",
            pattern: r"[3-6]\d{3}(?:[ -]?\d{4}){2,3}(?:[ -]?\d{1,3})?",
            bound: ALNUM_STR,
            ok: Some(luhn),
            cat: more::CAT_PERSONAL,
            ..Rule::base()
        },
        // ===== more.go（moreRules）=====
        // 美国社保号：123-45-6789 书写形式（在 SSA 发放的范围内）
        Rule {
            kind: "SSN",
            pattern: r"\d{3}-\d{2}-\d{4}",
            markers: &["-"],
            bound: BOUND_ALNUM_HYPHEN,
            ok: Some(more::ssn_ok),
            cat: "ssn",
            ..Rule::base()
        },
        // SSN 标注后九位数字
        Rule {
            kind: "SSN",
            pattern: r#"(?i:\bssn|\bsocial[ \t]+security(?:[ \t]+(?:number|no\.?|#))?)["']?[ \t]*(?:[:#=：][ \t]*)?(?:(?i:is)[ \t]+)?["']?(\d{9}|\d{3} \d{2} \d{4})"#,
            fold: &["ssn", "social"],
            back: 8,
            ahead: 80,
            bound: DIGITS_STR,
            ok: Some(more::ssn_ok),
            cat: "ssn",
            ..Rule::base()
        },
        // 护照号（英文/德文/中日文标注）
        Rule {
            kind: "PASSPORT",
            pattern: r#"(?i:\bpassport(?:[ \t]*(?:no\.?|number|num|#))?|\breisepass(?:nummer)?)["']?[ \t]*(?:[:#=：][ \t]*)?(?:(?i:is|was)[ \t]+)?["']?([A-Z0-9]{6,9})|(?:护照(?:号码|号)?|護照(?:號碼|号码|番号)?|パスポート(?:番号)?)[ \t]*(?:[:：=][ \t]*)?(?:是[ \t]*|は[ \t]*)?["']?([A-Z0-9]{6,9})"#,
            fold: &["passport", "reisepass", "护照", "護照", "パスポート"],
            back: 8,
            ahead: 80,
            bound: ALNUM_STR,
            ok: Some(more::passport_ok),
            any_group: true,
            cat: "passport",
            ..Rule::base()
        },
        // IBAN：整词或四位一组，按 mod 97 校验（fix 去掉误入末组的词）
        Rule {
            kind: "IBAN",
            pattern: r"[A-Z]{2}\d{2}(?:[A-Z0-9]{11,30}|(?: [A-Z0-9]{4}){2,7}(?: [A-Z0-9]{1,3})?)",
            bound: ALNUM_STR,
            fix: Some(fix_iban_at),
            cat: "iban",
            ..Rule::base()
        },
        // 生日：只在生日/DOB/born 等词后的日期；一年内的日期不是
        Rule {
            kind: "BIRTHDAY",
            pattern: r#"(?:(?i:\bdate[ \t]+of[ \t]+birth|\bbirth[ \t_-]?date|\bbirthday|\bdob\b|\bd\.o\.b\.?|\bborn(?:[ \t]+on)?|\bgeburtsdatum|\bgeboren(?:[ \t]+am)?)|生日|出生日期|出生年月日|出生于|出生於|出生|誕生日|生年月日)["']?[ \t]*(?:[:#=：][ \t]*)?(?:(?i:is|was)[ \t]+|是[ \t]*|为[ \t]*|為[ \t]*|は[ \t]*)?["']?(\d{4}[-/.]\d{1,2}[-/.]\d{1,2}|\d{4}[ \t]*年[ \t]*\d{1,2}[ \t]*月[ \t]*\d{1,2}[ \t]*日?|\d{1,2}[-/.]\d{1,2}[-/.]\d{4}|(?i:jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\.?[ \t]+\d{1,2}(?:st|nd|rd|th)?,?[ \t]+\d{4}|\d{1,2}(?:st|nd|rd|th)?[ \t]+(?i:jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\.?,?[ \t]+\d{4})"#,
            fold: &[
                "birth",
                "dob",
                "d.o.b",
                "born",
                "geburt",
                "geboren",
                "生日",
                "出生",
                "誕生日",
                "生年月日",
            ],
            back: 16,
            ahead: 80,
            bound: DIGITS_STR,
            ok: Some(more::date_ok),
            cat: "birthday",
            ..Rule::base()
        },
        // MAC 地址（不是 IPv6 片段或证书指纹）
        Rule {
            kind: "MAC",
            pattern: r"[0-9A-Fa-f]{2}(?::[0-9A-Fa-f]{2}){5}|[0-9A-Fa-f]{2}(?:-[0-9A-Fa-f]{2}){5}",
            bound: BOUND_MAC,
            ok: Some(more::mac_ok),
            cat: "mac",
            ..Rule::base()
        },
        // 序列号（S/N、Serial、序列号；不是 serial port / Serial.println）
        Rule {
            kind: "SERIAL",
            pattern: r#"(?i:\bserial[ \t_-]?(?:number|num|no\.?|#)?|\bs/n|\bsn)(?:[ \t]*\([^)\n]{1,20}\))?["']?[ \t]*[:#=：][ \t]*["']?([A-Za-z0-9][A-Za-z0-9-]{4,39})|(?:序列号|序列號|シリアル番号|(?i:seriennummer))[ \t]*[:：=]?[ \t]*["']?([A-Za-z0-9][A-Za-z0-9-]{4,39})"#,
            fold: &["serial", "s/n", "sn", "序列", "シリアル", "seriennummer"],
            back: 8,
            ahead: 80,
            bound: BOUND_ALNUM_HYPHEN,
            ok: Some(more::serial_ok),
            any_group: true,
            cat: "serial",
            ..Rule::base()
        },
        // 家目录里的用户名：/Users/<name>、/home/<name>（路径其余部分保留）
        Rule {
            kind: "USERNAME",
            pattern: r#"(?:^|[^A-Za-z0-9_.~/\\-])(?:file://)?(?:/mnt/[a-z])?/(?:Users|home)/([^\s/\\:*?"'<>|,;()\[\]{}`$%]{1,64})"#,
            fold: &["/users/", "/home/"],
            back: 16,
            ahead: 80,
            fix: Some(fix_home_at),
            cat: "home",
            ..Rule::base()
        },
        // Windows 家目录：C:\Users\<name>（含带空格的名字）
        Rule {
            kind: "USERNAME",
            pattern: r#"(?:^|[^A-Za-z0-9])[A-Za-z]:(?:\\{1,4}|/)(?i:users|documents and settings)(?:\\{1,4}|/)(?:([A-Za-z0-9._-]+(?: [A-Za-z0-9._-]+)+)(?:\\|/)|([^\s/\\:*?"'<>|,;()\[\]{}`$%]{1,64}))"#,
            fold: &["users", "documents and settings"],
            back: 12,
            ahead: 100,
            fix: Some(fix_home_at),
            any_group: true,
            cat: "home",
            ..Rule::base()
        },
        // user@host：shell 提示符形态（user@host:~$、[user@host ~]$）；
        // js-tokens@4.0.0 这类包版本锁定不是登录（notPackagePin）
        Rule {
            pattern: r#"(?m)(?:^|[^A-Za-z0-9._%+-])([a-z_][a-z0-9_.-]{0,31})@([A-Za-z0-9][A-Za-z0-9-]{0,62}(?:\.[A-Za-z0-9-]{1,63})*)(?::[~/]|:[ \t]*$|[ \t]+[~/]|[ \t]*[$%#>][ \t]|[ \t]*[$%#>]$)"#,
            groups: &["USERNAME", "HOSTNAME"],
            group_ok: &[more::user_ok, more::host_ok],
            markers: &["@"],
            at: Some(at_not_package_pin),
            cat: "userhost",
            ..Rule::base()
        },
        // ssh/scp/rsync 命令行里的 user@host
        Rule {
            pattern: r#"(?i:\b(?:ssh|scp|sftp|rsync|mosh|ssh-copy-id))\b[^\n@]{0,80}?[ \t"']([a-z_][a-z0-9_.-]{0,31})@([A-Za-z0-9][A-Za-z0-9-]{0,62}(?:\.[A-Za-z0-9-]{1,63})*)"#,
            groups: &["USERNAME", "HOSTNAME"],
            group_ok: &[more::user_ok, more::host_ok],
            markers: &["@"],
            at: Some(at_not_package_pin),
            cat: "userhost",
            ..Rule::base()
        },
        // hostname: x / HOSTNAME=x 单独成行；ssh config 缩进的 HostName
        Rule {
            kind: "HOSTNAME",
            pattern: r#"(?m)^[ \t]*["']?(?:(?i:host[ \t_-]?name|computer[ \t_-]?name|machine[ \t_-]?name|device[ \t_-]?name)|主机名|主機名|計算機名|コンピュータ名|コンピューター名)["']?[ \t]*[:=：][ \t]*(?:["']([A-Za-z0-9][A-Za-z0-9-]{0,62}(?:\.[A-Za-z0-9-]{1,63})*)["'][ \t]*[,;]?|([A-Za-z0-9][A-Za-z0-9-]{0,62}(?:\.[A-Za-z0-9-]{1,63})*))[ \t]*$|^[ \t]+HostName[ \t]+([A-Za-z0-9][A-Za-z0-9-]{0,62}(?:\.[A-Za-z0-9-]{1,63})*)[ \t]*$"#,
            fold: &["name", "主机", "主機", "計算機", "コンピュータ"],
            any_group: true,
            ok: Some(more::line_host_ok),
            cat: "userhost",
            ..Rule::base()
        },
        // User: x / USER=x 单独成行；ssh config 缩进的 User
        Rule {
            kind: "USERNAME",
            pattern: r#"(?m)^[ \t]*["']?(?:(?i:user[ \t_-]?name|login[ \t_-]?name|user|login|logname)|用户名|用戶名|ユーザー名)["']?[ \t]*[:=：][ \t]*(?:["']([A-Za-z0-9_][A-Za-z0-9._-]{0,31})["'][ \t]*[,;]?|([A-Za-z0-9_][A-Za-z0-9._-]{0,31}))[ \t]*$|^[ \t]+User[ \t]+([A-Za-z0-9_][A-Za-z0-9._-]{0,31})[ \t]*$"#,
            fold: &["user", "login", "logname", "用户", "用戶", "ユーザー"],
            any_group: true,
            ok: Some(more::line_user_ok),
            cat: "userhost",
            ..Rule::base()
        },
        // 公网 IPv4（非私有/环回/链路本地/CGNAT/文档段，非版本号）
        Rule {
            kind: "IP",
            pattern: r"(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(?:\.(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3}",
            markers: &["."],
            at: Some(at_ipv4),
            cat: "ip",
            ..Rule::base()
        },
        // 公网 IPv6
        Rule {
            kind: "IP",
            pattern: r"[0-9A-Fa-f]{1,4}(?::[0-9A-Fa-f]{0,4}){2,7}",
            markers: &[":"],
            bound: BOUND_IP6,
            ok: Some(more::ipv6_ok),
            cat: "ip",
            ..Rule::base()
        },
        // 云桶：s3://<b>、gs://<b>、abfss://<c>@<account>… 等
        Rule {
            pattern: r#"(?i:\b(?:s3a?|s3n|gs|oss|cosn?|obs|r2|ks3|bos|tos|az|azure|wasbs?|abfss?)://)([A-Za-z0-9](?:[A-Za-z0-9._-]{0,61}[A-Za-z0-9])?)(?:@([a-z0-9]{3,24})\.(?:blob|dfs)\.core\.windows\.net)?"#,
            groups: &["BUCKET", "BUCKET"],
            group_ok: &[more::bucket_ok, more::bucket_ok],
            markers: &["://"],
            cat: "bucket",
            ..Rule::base()
        },
        // <b>.s3.amazonaws.com 等主机前缀形态的桶
        Rule {
            kind: "BUCKET",
            pattern: r#"(?:^|[^A-Za-z0-9.-])([a-z0-9](?:[a-z0-9.-]{0,61}[a-z0-9])?)\.(?:s3(?:[.-](?:dualstack\.)?[a-z0-9-]+)?\.amazonaws\.com(?:\.cn)?|s3-accelerate\.amazonaws\.com|storage\.googleapis\.com|oss-[a-z0-9-]+\.aliyuncs\.com|cos\.[a-z0-9-]+\.myqcloud\.com|obs\.[a-z0-9-]+\.myhuaweicloud\.com|r2\.cloudflarestorage\.com|(?:blob|dfs)\.core\.windows\.net|[a-z0-9-]+\.digitaloceanspaces\.com|tos-[a-z0-9-]+\.volces\.com)"#,
            fold: BUCKET_HOSTS,
            back: 128,
            ahead: 32,
            ok: Some(more::bucket_ok),
            cat: "bucket",
            ..Rule::base()
        },
        // s3.amazonaws.com/<b> 等路径形态的桶
        Rule {
            kind: "BUCKET",
            pattern: r#"(?:^|[^A-Za-z0-9.-])(?:s3(?:[.-](?:dualstack\.)?[a-z0-9-]+)?\.amazonaws\.com(?:\.cn)?|storage\.googleapis\.com|storage\.cloud\.google\.com|[a-z0-9]{3,24}\.blob\.core\.windows\.net|oss-[a-z0-9-]+\.aliyuncs\.com|[a-z0-9-]+\.digitaloceanspaces\.com)/([A-Za-z0-9](?:[A-Za-z0-9._-]{0,61}[A-Za-z0-9])?)"#,
            fold: BUCKET_HOSTS,
            back: 48,
            ahead: 80,
            ok: Some(more::bucket_ok),
            cat: "bucket",
            ..Rule::base()
        },
    ]
}

// ---- Options ----

/// 遮盖选项（Magpie `Options`）。
#[derive(Clone, Default)]
pub struct MaskOptions {
    pub secrets: bool,
    pub personal: bool,
    pub words: Vec<String>,
    pub words_any_case: bool,
    pub words_whole: bool,
    /// 用户自定义规则（rules.rs）。
    pub custom_rules: Vec<super::rules::UserRule>,
    /// 类别逐项开关。
    pub kinds: HashMap<String, bool>,
    /// Shape 模式：密钥走合成键。
    pub shape: bool,
    /// E-08 中文姓名检测（privacy_detect_cjk_names，默认关闭）。
    pub detect_cjk_names: bool,
}

impl MaskOptions {
    /// 已知值再匹配的种类过滤（Magpie 只看 secrets）。我方偏差（E-08）：
    /// PERSON 只在姓名开关打开时再匹配——开关关闭后不应继续遮人名。
    pub(crate) fn known_covers(&self, kind: &str) -> bool {
        self.secrets && (kind != "PERSON" || (self.personal && self.detect_cjk_names))
    }

    fn covers(&self, cat: &str) -> bool {
        match cat {
            "" => self.secrets,
            more::CAT_PERSONAL => self.personal,
            _ => {
                if !self.personal {
                    return false;
                }
                if let Some(on) = self.kinds.get(cat) {
                    return *on;
                }
                more::category_on(cat)
            }
        }
    }
}

// ---- mask 引擎 ----

/// Mask（Magpie `Mask`）：对 s 按 o 遮盖，返回结果与替换数。
pub fn mask(s: &str, o: &MaskOptions, engine: &mut PlaceholderEngine) -> (String, usize) {
    mask_with(s, o, engine, None)
}

/// mask 带记录：每次遮盖把 (值, 占位符) 推进 record（walker 的封签块
/// 重遮用）。
pub(crate) fn mask_with(
    s: &str,
    o: &MaskOptions,
    engine: &mut PlaceholderEngine,
    record: Option<&mut Vec<(String, String)>>,
) -> (String, usize) {
    if s.len() < 3 {
        return (s.to_string(), 0);
    }
    let found = scan(s, o);
    apply_with(s, found, o, engine, record, false)
}

/// mask 的 escaped 变体（Magpie `mask(escaped=true)`）：raw 是一段 JSON
/// 文本（工具调用的 arguments）—— 查找器读解开一层转义后的视图，找到的
/// 区间映射回 raw（outsideEscapes + inStrings），值解开转义后入表。
pub(crate) fn mask_escaped(
    raw: &str,
    o: &MaskOptions,
    engine: &mut PlaceholderEngine,
    record: Option<&mut Vec<(String, String)>>,
) -> (String, usize) {
    if raw.len() < 3 {
        return (raw.to_string(), 0);
    }
    let view = crate::aicli::privacy::walker::unescaped_view(raw);
    let found = scan(&view, o);
    let found = crate::aicli::privacy::walker::in_strings(
        raw,
        crate::aicli::privacy::walker::outside_escapes(raw, found),
    );
    apply_with(raw, found, o, engine, record, true)
}

/// scan（Magpie `mask` 内的查找部分）：返回全部找到的 Span。
pub(crate) fn scan(s: &str, o: &MaskOptions) -> Vec<Span> {
    let mut found = Vec::new();

    // 用户词
    for w in &o.words {
        let w = w.trim();
        if w.len() < 2 {
            continue;
        }
        // 日期词
        if let Some(spans) = dateword::date_spans(s, w) {
            found.extend(spans);
            continue;
        }
        // 精确查找（简化版；Magpie 还有 WordsAnyCase 与 WordsWhole）
        let mut i = 0;
        while let Some(j) = s[i..].find(w) {
            let a = i + j;
            let b = a + w.len();
            found.push(Span {
                start: a,
                end: b,
                kind: "TERM",
                by: FoundBy::Word,
                bare: false,
            });
            i = b;
        }
    }

    // 规则（Magpie mask 的规则循环）：markers 过滤、fold 过滤 + 窗口匹配、
    // groups 拆分遮盖（user@host 各自为值）、anyGroup、fix、takes
    let mut lower = String::new();
    let mut have_lower = false;
    let mut split: Vec<(usize, usize)> = Vec::new();
    for r in core_rules() {
        if !o.covers(r.cat) {
            continue;
        }
        if !r.markers.is_empty() && !r.markers.iter().any(|m| s.contains(m)) {
            continue;
        }
        if !r.fold.is_empty() {
            if !have_lower {
                lower = lower_ascii(s);
                have_lower = true;
            }
            if !r.fold.iter().any(|f| lower.contains(f)) {
                continue;
            }
        }
        for m in r.matches(s, &lower) {
            if !r.groups.is_empty() {
                // 组规则（Magpie groups/groupOK）：每个匹配的组按 groups 的
                // 类型分别遮盖；任一组成立则整段记为 split
                let mut took = false;
                for (g, kind) in r.groups.iter().enumerate() {
                    if let Some(Some((a, b))) = m.get(g + 1) {
                        if r.takes(s, *a, *b, r.group_ok.get(g).copied()) {
                            found.push(Span {
                                start: *a,
                                end: *b,
                                kind,
                                by: r.by(),
                                bare: false,
                            });
                            took = true;
                        }
                    }
                }
                if took {
                    if let Some(Some((wa, _))) = m.get(1) {
                        split.push((*wa, m[0].unwrap().1));
                    }
                }
                continue;
            }
            // 默认取整段匹配；组 1 匹配则取组 1；anyGroup 时向后找第一个
            // 匹配的组（多分支各有组的模式）
            let (mut a, mut b) = m[0].unwrap();
            for item in m.iter().skip(1) {
                if let Some((ga, gb)) = item {
                    a = *ga;
                    b = *gb;
                    break;
                }
                if !r.any_group {
                    break;
                }
            }
            let mut ok = r.ok;
            if let Some(fix) = r.fix {
                match fix(s, a, b) {
                    Some((fa, fb)) => {
                        a = fa;
                        b = fb;
                        ok = None; // fix 已校验，不再跑 ok（Magpie 同）
                    }
                    None => continue,
                }
            }
            if r.takes(s, a, b, ok) {
                found.push(Span {
                    start: a,
                    end: b,
                    kind: r.kind,
                    by: r.by(),
                    bare: false,
                });
            }
        }
    }
    // 拆分遮盖的 user@host 段内不再按 EMAIL 重复遮盖（Magpie split）
    for (ss, se) in &split {
        found.retain(|f| !(f.kind == "EMAIL" && f.start == *ss && f.end <= *se));
    }

    // 中文姓名（E-08，Magpie 的 zhNameSpans；等价 person=[]、nicks=false，
    // 仅在 personal 开启且开关打开时生效）
    if o.personal && o.detect_cjk_names {
        found.extend(super::zhname::zh_name_spans(s, &[], false));
    }

    // 已知值
    if o.secrets || o.personal {
        let idx = known::shared_index()
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let known_spans = idx.spans(s, |kind| o.known_covers(kind));
        found.extend(known_spans);
    }

    // 过滤：id 片段、数字片段
    let bytes = s.as_bytes();
    found = ids::no_ids(bytes, found);
    found
}

/// apply（Magpie `apply`）：把 found 的值替换为占位符。
pub(crate) fn apply(
    s: &str,
    found: Vec<Span>,
    o: &MaskOptions,
    engine: &mut PlaceholderEngine,
) -> (String, usize) {
    apply_with(s, found, o, engine, None, false)
}

/// apply 带记录与转义值（Magpie apply + unescapedPut）。
fn apply_with(
    s: &str,
    found: Vec<Span>,
    o: &MaskOptions,
    engine: &mut PlaceholderEngine,
    mut record: Option<&mut Vec<(String, String)>>,
    unescape_values: bool,
) -> (String, usize) {
    if found.is_empty() {
        return (s.to_string(), 0);
    }
    let mut found = found;
    found.sort_by_key(|f| (f.start, std::cmp::Reverse(f.end)));

    let mut out = String::with_capacity(s.len());
    let mut last = 0;
    let mut count = 0;

    for f in &found {
        if f.start < last {
            continue; // 重叠
        }
        let raw_value = &s[f.start..f.end];
        if raw_value.is_empty() {
            continue;
        }
        // unescapedPut：escaped JSON 文本里的值带转义，入表前解开
        let unescaped;
        let value = if unescape_values && raw_value.contains('\\') {
            unescaped = serde_json::from_slice::<String>(format!("\"{raw_value}\"").as_bytes())
                .unwrap_or_else(|_| raw_value.to_string());
            unescaped.as_str()
        } else {
            raw_value
        };
        out.push_str(&s[last..f.start]);
        let kind = if f.kind.is_empty() { "SECRET" } else { f.kind };
        let placeholder = if o.shape {
            // Shape 模式暂用占位符（完整实现需 shape 模块协作）
            engine.mask(kind, value)
        } else {
            engine.mask(kind, value)
        };
        if let Some(rec) = record.as_deref_mut() {
            rec.push((value.to_string(), placeholder.clone()));
        }
        if f.bare {
            // inStrings 的裸值：占位符带引号放回
            out.push('"');
            out.push_str(&placeholder);
            out.push('"');
        } else {
            out.push_str(&placeholder);
        }
        last = f.end;
        count += 1;
    }
    out.push_str(&s[last..]);
    (out, count)
}

#[cfg(test)]
mod tests;
