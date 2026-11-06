use super::*;
use crate::aicli::secrets::{MemoryStore, SecretStore};
use serde::Deserialize;

/// 与 tools/magpie_golden/main.go 的 fixedKeyHex 一致。
const FIXED_KEY_HEX: &str = "7f3a9c1d5e2b8406ad17fe93c2b60845d1a4f7e29b3c5d8a06f4e7b2c9d1a305";

fn fixed_key() -> [u8; KEY_LEN] {
    let mut key = [0u8; KEY_LEN];
    for (index, slot) in key.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&FIXED_KEY_HEX[index * 2..index * 2 + 2], 16).unwrap();
    }
    key
}

#[derive(Deserialize)]
struct GoldenEntry {
    kind: String,
    value: String,
    placeholder: String,
}

/// 验收：Magpie（Go，固定密钥）对语料生成的占位符与 Rust 完全一致。
/// golden 由 `go run ./tools/magpie_golden` 生成（见工具注释）。
#[test]
fn golden_placeholders_match_magpie_exactly() {
    let golden: Vec<GoldenEntry> = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/privacy/golden_placeholder.json"
    ))
    .expect("golden fixture parses");
    assert!(
        golden.len() >= 5,
        "golden corpus too small: {}",
        golden.len()
    );
    let engine = PlaceholderEngine::with_key(fixed_key());
    for entry in &golden {
        assert_eq!(
            engine.tag(&entry.kind, &entry.value),
            entry.placeholder,
            "kind={} value mismatch",
            entry.kind
        );
        // 占位符形态自查。
        assert!(
            entry
                .placeholder
                .starts_with(&format!("{{{{{}}}_", entry.kind)),
            "kind prefix missing: {}",
            entry.placeholder
        );
    }
}

/// 占位符正则：合法形态命中，相似但不合规的形态不命中（大写后缀、9 位、小写 kind）。
#[test]
fn placeholder_pattern_shape() {
    let re = regex::Regex::new(PLACEHOLDER_PATTERN).unwrap();
    assert!(re.is_match("{{EMAIL_abcd2345}}"));
    assert!(re.is_match("x {{PHONE_2a2b2c2d}} y"));
    assert!(re.is_match("{{API_KEY_q2w3e4r5}}"));
    assert!(!re.is_match("{{email_abcd2345}}"), "kind must be upper");
    assert!(
        !re.is_match("{{EMAIL_ABCD2345}}"),
        "suffix must be lower/base32"
    );
    assert!(!re.is_match("{{EMAIL_abcd23456}}"), "suffix is 8 chars");
    assert!(!re.is_match("{{EMAIL_abcd234}}"), "suffix is 8 chars");
    assert!(
        !re.is_match("{{1EMAIL_abcd2345}}"),
        "kind starts with a letter"
    );
}

/// 同值同占位符；模拟重启（新值表、同密钥）后不变。
#[test]
fn same_value_same_placeholder_across_restart() {
    let key = fixed_key();
    let mut first = PlaceholderEngine::with_key(key);
    let placeholder = first.mask("EMAIL", "anchor@example.com");
    assert_eq!(first.restore(&placeholder), Some("anchor@example.com"));
    let again = first.mask("EMAIL", "anchor@example.com");
    assert_eq!(again, placeholder, "repeat mask is stable");

    // 重启：新值表、同密钥。
    let mut second = PlaceholderEngine::with_key(key);
    assert_eq!(second.restore(&placeholder), None, "table is memory only");
    assert_eq!(
        second.mask("EMAIL", "anchor@example.com"),
        placeholder,
        "placeholder survives a restart"
    );
    assert_eq!(second.restore(&placeholder), Some("anchor@example.com"));
    // kind 不同则占位符不同。
    assert_ne!(second.tag("PHONE", "anchor@example.com"), placeholder);
}

/// 淘汰：表满丢最旧一半；被淘汰的值再次出现时得到同一占位符并可恢复。
#[test]
fn eviction_drops_oldest_half_and_recovers() {
    let mut engine = PlaceholderEngine::with_key(fixed_key()).with_limit_for_test(4);
    let first = engine.mask("EMAIL", "old@example.com");
    for index in 0..3 {
        engine.mask("EMAIL", &format!("v{index}@example.com"));
    }
    assert_eq!(engine.len(), 4);
    // 触发淘汰：最旧的 first 被丢，表剩 2 + 新 1。
    let fresh = engine.mask("EMAIL", "new@example.com");
    assert_eq!(engine.len(), 3, "evicted half then inserted");
    assert_eq!(engine.restore(&first), None, "oldest evicted");
    assert_eq!(engine.restore(&fresh), Some("new@example.com"));
    // 淘汰后原值再次出现：同一占位符，且重新可恢复。
    let recovered = engine.mask("EMAIL", "old@example.com");
    assert_eq!(recovered, first, "deterministic after eviction");
    assert_eq!(engine.restore(&first), Some("old@example.com"));
}

/// Keychain 语义：首次生成 32 字节随机密钥并写回；再次构造读同一密钥。
#[test]
fn key_is_generated_once_and_persisted() {
    let store = MemoryStore::default();
    let first = PlaceholderEngine::from_store(&store).expect("first engine");
    let stored = store.get(KEY_ACCOUNT).unwrap().expect("key stored");
    assert_eq!(stored.len(), KEY_LEN * 2, "hex encoded 32 bytes");
    let second = PlaceholderEngine::from_store(&store).expect("second engine");
    assert_eq!(
        first.tag("EMAIL", "same@example.com"),
        second.tag("EMAIL", "same@example.com"),
        "same key across engines"
    );
    let other = PlaceholderEngine::with_key(fixed_key());
    assert_ne!(
        first.tag("EMAIL", "same@example.com"),
        other.tag("EMAIL", "same@example.com"),
        "random key differs from the fixture key"
    );
}
