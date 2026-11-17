//! 用户自定义规则（Magpie `rules.go`，@f11bf27）。
//!
//! 用户可添加前缀规则或正则规则（至多 32 条），种类名归一化为占位符可用的
//! 形态。值经引擎统一遮盖。

use regex::Regex;
use serde::{Deserialize, Serialize};

/// 限制常量（Magpie）。
pub const MAX_RULES: usize = 32;
const MAX_KIND: usize = 24;
const MAX_PREFIX: usize = 64;
const MIN_PREFIX: usize = 3;
const MAX_REGEX: usize = 300;
const PREFIX_TAIL: &str = r"[A-Za-z0-9_\-]{8,}";
const CUSTOM_KIND: &str = "CUSTOM";

/// 一条用户规则（Magpie `Rule`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserRule {
    pub kind: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub prefix: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub regex: String,
}

/// RuleKind：种类名归一化为占位符可用形态（Magpie `RuleKind`）。
pub fn rule_kind(kind: &str) -> String {
    let mut b = String::new();
    for r in kind.trim().to_uppercase().chars() {
        if r.is_ascii_uppercase() || r.is_ascii_digit() || r == '_' {
            b.push(r);
        } else if r == ' ' || r == '-' || r == '.' {
            b.push('_');
        }
    }
    let k = b.trim_matches('_').to_string();
    if k.is_empty() {
        return CUSTOM_KIND.to_string();
    }
    let mut k = k;
    if !k.starts_with(|c: char| c.is_ascii_uppercase()) {
        k = format!("{CUSTOM_KIND}_{k}");
    }
    if k.len() > MAX_KIND {
        k = k[..MAX_KIND].trim_end_matches('_').to_string();
    }
    k
}

/// KindName：k 是否已可用作占位符种类名。
pub fn kind_name(k: &str) -> bool {
    !k.is_empty() && rule_kind(k) == k
}

impl UserRule {
    /// compile：把规则编译为正则（Magpie `Rule.compile`）。
    pub fn compile(&self) -> Result<Regex, String> {
        if !self.prefix.is_empty() {
            return Regex::new(&format!("{}{}", regex::escape(&self.prefix), PREFIX_TAIL))
                .map_err(|e| e.to_string());
        }
        Regex::new(&self.regex).map_err(|e| e.to_string())
    }
}

/// CheckRules：整理并验证用户规则（Magpie `CheckRules`）。
pub fn check_rules(input: &[UserRule]) -> Result<Vec<UserRule>, String> {
    let mut out = Vec::new();
    for r in input {
        let mut r = UserRule {
            kind: r.kind.trim().to_string(),
            prefix: r.prefix.trim().to_string(),
            regex: r.regex.trim().to_string(),
        };
        if r.prefix.is_empty() && r.regex.is_empty() {
            continue;
        }
        r.kind = rule_kind(&r.kind);
        if !r.prefix.is_empty() && !r.regex.is_empty() {
            return Err(format!(
                "a masking rule has a prefix or a regular expression, not both: {}",
                r.kind
            ));
        }
        if !r.prefix.is_empty() && (r.prefix.len() < MIN_PREFIX || r.prefix.len() > MAX_PREFIX) {
            return Err(format!(
                "prefix must be {MIN_PREFIX} to {MAX_PREFIX} characters, not {:?}",
                r.prefix
            ));
        }
        if r.regex.len() > MAX_REGEX {
            return Err(format!("regex at most {MAX_REGEX} characters"));
        }
        let re = r
            .compile()
            .map_err(|e| format!("rule {} has a regex that doesn't work: {e}", r.kind))?;
        if re.is_match("") {
            return Err(format!(
                "rule {} matches nothing (empty match), would mask everywhere",
                r.kind
            ));
        }
        if !out.contains(&r) {
            out.push(r);
        }
    }
    if out.len() > MAX_RULES {
        return Err(format!("at most {MAX_RULES} rules, not {}", out.len()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_kind_normalizes() {
        assert_eq!(rule_kind("API_KEY"), "API_KEY");
        assert_eq!(rule_kind("my secret"), "MY_SECRET");
        assert_eq!(rule_kind("x-key.v2"), "X_KEY_V2");
        assert_eq!(rule_kind(""), "CUSTOM");
        assert_eq!(rule_kind("123"), "CUSTOM_123");
        let long = "A".repeat(40);
        assert!(rule_kind(&long).len() <= MAX_KIND);
    }

    #[test]
    fn kind_name_roundtrip() {
        assert!(kind_name("API_KEY"));
        assert!(!kind_name("api_key"));
        assert!(!kind_name(""));
        assert!(kind_name("CUSTOM"));
    }

    #[test]
    fn prefix_and_regex_mutually_exclusive() {
        let rules = vec![UserRule {
            kind: "TEST".into(),
            prefix: "pref".into(),
            regex: "abc\\d+".into(),
        }];
        assert!(check_rules(&rules).is_err());
    }

    #[test]
    fn valid_rules_pass() {
        let rules = vec![
            UserRule {
                kind: "MY_KEY".into(),
                prefix: "mykey_".into(),
                regex: String::new(),
            },
            UserRule {
                kind: "TOKEN".into(),
                prefix: String::new(),
                regex: "tok_[a-z0-9]+".into(),
            },
        ];
        let out = check_rules(&rules).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out[0].compile().is_ok());
        assert!(out[1].compile().is_ok());
    }

    #[test]
    fn empty_match_rejected() {
        let rules = vec![UserRule {
            kind: "BAD".into(),
            prefix: String::new(),
            regex: "a*b*".into(), // matches empty string
        }];
        assert!(check_rules(&rules).is_err());
    }

    #[test]
    fn duplicate_dropped() {
        let rules = vec![
            UserRule {
                kind: "K".into(),
                prefix: "abc_".into(),
                regex: String::new(),
            },
            UserRule {
                kind: "K".into(),
                prefix: "abc_".into(),
                regex: String::new(),
            },
        ];
        assert_eq!(check_rules(&rules).unwrap().len(), 1);
    }
}
