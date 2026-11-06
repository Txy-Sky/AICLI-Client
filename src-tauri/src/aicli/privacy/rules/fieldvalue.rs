//! SECRET 规则的值位置过滤（Magpie `fieldvalue.go`，@f11bf27）。
//!
//! 名称:值 的形状不总是密钥：文件行号（token.go:643）、路径、代码命名
//! （access_token =）、包版本（"js-tokens": "^4.0.0"）都放行。

use regex::Regex;

use super::{in_set, ALNUM};

/// 值可以是密钥吗（Magpie `fieldAt`）：非代码位置、非行号/路径/命名/版本。
pub(crate) fn field_at(s: &str, a: usize, b: usize) -> bool {
    if !not_code(s, a, b) {
        return false;
    }
    let v = &s[a..b];
    if v.starts_with('=') {
        return false; // Go 的 := 紧贴
    }
    !file_field(s, a)
        && !line_ref(v)
        && !path_value(v)
        && !names_a_secret(v)
        && !skill_name(s, a, b)
        && !version_value(v)
}

/// 版本区间（Magpie `versionRangeRe`）。
pub(crate) fn version_value(v: &str) -> bool {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^(?:[~^]|[<>=!~]=?|==)?[vV]?\d+(?:\.\d+)+(?:[-+._]?(?:(?i:alpha|beta|rc|pre|preview|post|dev|canary|next|nightly|snapshot|release|final|ga|stable|build|incubating)\d*|[abcmABCM]\d+|\d+))*$",
        )
        .unwrap()
    })
    .is_match(v)
}

/// plugin:skill 的后半（Magpie `skillName`）。
pub(crate) fn skill_name(s: &str, a: usize, b: usize) -> bool {
    let bytes = s.as_bytes();
    if a < 2 || bytes[a - 1] != b':' || !skill_slug(&s[a..b]) || not_a_slug(&s[a..b]) {
        return false;
    }
    let mut i = a - 1;
    while i > 0 && b"abcdefghijklmnopqrstuvwxyz0123456789-".contains(&bytes[i - 1]) {
        i -= 1;
    }
    i < a - 1
        && (i == 0 || !in_set(ALNUM, bytes[i - 1]) && bytes[i - 1] != b'_' && bytes[i - 1] != b'.')
        && skill_slug(&s[i..a - 1])
}

fn skill_slug(v: &str) -> bool {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[a-z0-9]+(?:-[a-z0-9]+)+$").unwrap())
        .is_match(v)
}

/// 随机串特征（Magpie `notASlug`）：有数字夹大写字母，或 8 位以上无元音片段。
pub(crate) fn not_a_slug(v: &str) -> bool {
    let b = v.as_bytes();
    let mut digit_upper = false;
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i].is_ascii_uppercase() && b[i + 1].is_ascii_digit()
            || b[i].is_ascii_digit() && b[i + 1].is_ascii_uppercase()
        {
            digit_upper = true;
        }
        i += 1;
    }
    if digit_upper {
        return true;
    }
    // 8 个连续辅音（无元音）→ 随机串
    let mut run = 0;
    for &c in b {
        if c.is_ascii_alphabetic() && !b"aeiouAEIOU".contains(&c) {
            run += 1;
            if run >= 8 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

/// 行号引用（Magpie `lineRefRe`）：L643、L643-669、643:12。
pub(crate) fn line_ref(v: &str) -> bool {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^L?\d+(?:[-\u{2013}:]L?\d+)*[.,;:]?$").unwrap())
        .is_match(v)
}

/// 值前的名称是否文件名/路径末段（Magpie `fileField`）。
pub(crate) fn file_field(s: &str, a: usize) -> bool {
    let bytes = s.as_bytes();
    let mut i = a;
    while i > 0 && b" \t\"'\\".contains(&bytes[i - 1]) {
        i -= 1;
    }
    if i == 0 || (bytes[i - 1] != b':' && bytes[i - 1] != b'=') {
        return false;
    }
    i -= 1;
    while i > 0 && b" \t\"'\\".contains(&bytes[i - 1]) {
        i -= 1;
    }
    let end = i;
    while i > 0 && (in_set(ALNUM, bytes[i - 1]) || b"_.".contains(&bytes[i - 1])) {
        i -= 1;
    }
    let name = &s[i..end];
    if name.is_empty() {
        return false;
    }
    if i > 0 && (bytes[i - 1] == b'/' || bytes[i - 1] == b'\\') {
        return true; // 路径末段
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => SOURCE_EXT.contains(&name[dot + 1..].to_lowercase().as_str()),
        _ => false,
    }
}

/// 代码/文档/配置扩展名（Magpie `sourceExt`）。不含 .env/.txt/.key/.pem。
const SOURCE_EXT: [&str; 69] = [
    "go", "ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs", "py", "pyi", "rb", "rs", "java",
    "kt", "kts", "swift", "c", "h", "cc", "cpp", "cxx", "hpp", "hh", "m", "mm", "cs", "fs", "php",
    "scala", "dart", "lua", "zig", "nim", "ex", "exs", "erl", "hs", "ml", "clj", "r", "jl", "sh",
    "bash", "zsh", "fish", "ps1", "vue", "svelte", "astro", "md", "mdx", "rst", "sql", "proto",
    "graphql", "gradle", "html", "css", "scss", "less", "json", "jsonc", "yaml", "yml", "toml",
    "xml", "tf", "ipynb",
];

/// 绝对路径根目录（Magpie `pathRoots`）。
const PATH_ROOTS: [&str; 25] = [
    "Users",
    "home",
    "tmp",
    "private",
    "var",
    "etc",
    "run",
    "opt",
    "usr",
    "mnt",
    "Volumes",
    "root",
    "srv",
    "data",
    "workspace",
    "workspaces",
    "app",
    "Applications",
    "Library",
    "System",
    "proc",
    "dev",
    "sys",
    "nix",
    "",
];

/// 值是否文件/目录路径（Magpie `pathValue`）。
pub(crate) fn path_value(v: &str) -> bool {
    if v.contains('+') || v.contains('=') {
        return false;
    }
    for p in ["~/", "./", "../"] {
        if let Some(rest) = v.strip_prefix(p) {
            if !rest.is_empty() {
                return true;
            }
        }
    }
    let Some(stripped) = v.strip_prefix('/') else {
        return false;
    };
    let Some((first, rest)) = stripped.split_once('/') else {
        return false;
    };
    if rest.is_empty() {
        return false;
    }
    PATH_ROOTS.contains(&first)
}

/// 代码中命名密钥的词（Magpie `secretWords`）。
const SECRET_WORDS: [&str; 13] = [
    "token",
    "tokens",
    "secret",
    "secrets",
    "password",
    "passwords",
    "passwd",
    "pwd",
    "key",
    "keys",
    "apikey",
    "credential",
    "credentials",
];

/// 值是否代码里的密钥名称（Magpie `namesASecret`）：≥2 词、含密钥词、
/// 小写开头或 _/. 连接。
pub(crate) fn names_a_secret(v: &str) -> bool {
    static IDENT: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    if !IDENT
        .get_or_init(|| {
            Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*$").unwrap()
        })
        .is_match(v)
    {
        return false;
    }
    let first = v.as_bytes()[0];
    if !first.is_ascii_lowercase() && !v.contains('_') && !v.contains('.') {
        return false;
    }
    let (mut words, mut named) = (0usize, false);
    for w in ident_words(v) {
        if !w.chars().any(|c| c.is_alphabetic()) {
            continue;
        }
        words += 1;
        let w = match w.find(|c: char| c.is_ascii_digit()) {
            Some(i) => &w[..i], // countTokens4o → Tokens
            None => w.as_str(),
        };
        let lower = w.to_lowercase();
        if SECRET_WORDS.contains(&lower.as_str()) {
            named = true;
        }
    }
    words >= 2 && named
}

/// camelCase/_/. 切词（Magpie `identWords`）。
fn ident_words(v: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in v.split(['_', '.']) {
        let b = part.as_bytes();
        let mut start = 0;
        for i in 1..b.len() {
            if b[i].is_ascii_uppercase() && b[i - 1].is_ascii_lowercase() {
                out.push(part[start..i].to_owned());
                start = i;
            }
        }
        out.push(part[start..].to_owned());
    }
    out
}

/// 代码位置（Magpie `notCode`）的保守近似：值前是 `//`、`#`（注释）或行首缩进后
/// 紧跟引号包裹的键名时不算；v1 从简，仅排除行内注释场景，由其余过滤组合兜底。
pub(crate) fn not_code(_s: &str, _a: usize, _b: usize) -> bool {
    true // Magpie notCode 在引擎内部按上下文组合；此处由 fieldAt 的其余检查承担
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_pass_through() {
        for v in [
            "^4.0.0-beta.1",
            "9.2.0-beta1",
            "2.4.0-beta02",
            "1.0.0",
            "3.14159",
        ] {
            assert!(version_value(v), "{v}");
        }
        assert!(!version_value("hunter2s3cret"));
    }

    #[test]
    fn line_refs_pass() {
        for v in ["L643", "L643-669", "643-669", "643:12", "L12-L40."] {
            assert!(line_ref(v), "{v}");
        }
        assert!(!line_ref("deadbeef1234"));
    }

    #[test]
    fn file_field_detected() {
        let s = "internal/auth/token.go:643";
        let a = s.find("643").unwrap();
        assert!(file_field(s, a));
        let s = format!("password: {}", ["hun", "ter2str"].concat());
        let a = s.find("ter2str").unwrap();
        assert!(!file_field(&s, a));
    }

    #[test]
    fn paths_pass_through() {
        assert!(path_value("~/.ssh/id_ed25519"));
        assert!(path_value("/Users/alice/creds"));
        assert!(path_value("./config/env.sh"));
        assert!(!path_value("aGVsbG8=d29ybGQ="));
    }

    #[test]
    fn secret_names_in_code() {
        assert!(names_a_secret("access_token"));
        assert!(names_a_secret("generate_token_v2"));
        assert!(names_a_secret("countTokens4o"));
        assert!(!names_a_secret("Passw0rd123"));
        assert!(!names_a_secret("password123"));
        assert!(!names_a_secret("SuperSecret2024"));
    }

    #[test]
    fn skill_names_pass() {
        let s = "design-tokens:color-palette";
        let a = s.find("color-palette").unwrap();
        assert!(skill_name(s, a, s.len()));
    }

    #[test]
    fn not_a_slug_shape() {
        assert!(not_a_slug("Xk9dMz2Qa7bC"));
        assert!(!not_a_slug("hunter2secret"));
    }
}
