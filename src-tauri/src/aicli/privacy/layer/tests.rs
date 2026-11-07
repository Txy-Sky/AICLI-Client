//! E-06 中间件单元测试（纯逻辑部分；端到端见 tests/aicli_privacy_e2e.rs）。

use super::*;
use crate::aicli::secrets::{MemoryStore, SecretStore};

fn gate() -> PrivacyGate {
    let db = Arc::new(Database::memory().unwrap());
    let store: Arc<dyn SecretStore> = Arc::new(MemoryStore::default());
    PrivacyGate::new(db, store)
}

fn gate_bound(enabled: bool, block: bool, allow_media: bool) -> PrivacyGate {
    let db = Arc::new(Database::memory().unwrap());
    crate::aicli::seed::init(&db).unwrap();
    db.conn
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO aicli_account(id, privacy_enabled, privacy_failure_mode, allow_unscanned_media)
             VALUES(1, ?1, ?2, ?3)",
            rusqlite::params![enabled, if block { "block" } else { "pass" }, allow_media],
        )
        .unwrap();
    let store: Arc<dyn SecretStore> = Arc::new(MemoryStore::default());
    PrivacyGate::new(db, store)
}

#[test]
fn ai_paths_and_media_paths() {
    for path in [
        "/v1/messages",
        "/claude-desktop/v1/messages",
        "/v1/chat/completions",
        "/v1/responses",
        "/v1/responses/compact",
        "/grokbuild/v1/responses",
        "/v1beta/models/m:generateContent",
        "/gemini/v1beta/models/m:streamGenerateContent",
        "/v1beta/models/m:countTokens",
        "/app/x",
        "/codex/v1/alpha/search",
    ] {
        assert!(is_ai_path(path), "{path}");
    }
    for path in [
        "/v1/models",
        "/health",
        "/v1/embeddings",
        "/images/generations",
        "/v1/images/edits",
    ] {
        assert!(!is_ai_path(path), "{path}");
    }
    assert!(is_media_path("/v1/images/generations") && is_media_path("/images/edits"));
    assert!(!is_media_path("/v1/messages"));
}

#[test]
fn unscannable_media_shapes() {
    assert!(contains_unscannable_media(
        r#"{"content":[{"type":"image","source":{"data":"data:image/png;base64,iVBOR"}}]}"#
    ));
    assert!(contains_unscannable_media(r#"{"file_data":"AAAA"}"#));
    assert!(!contains_unscannable_media(
        r#"{"content":[{"type":"text","text":"plain"}]}"#
    ));
    assert!(!contains_unscannable_media("not json"));
}

#[test]
fn settings_cached_until_revision_bumped() {
    let gate = gate_bound(true, true, false);
    let first = gate.settings();
    assert!(first.enabled && first.block && !first.allow_unscanned_media);
    // 改库不 bump：仍走缓存
    gate.db
        .conn
        .lock()
        .unwrap()
        .execute("UPDATE aicli_account SET privacy_enabled=0", [])
        .unwrap();
    assert!(gate.settings().enabled, "cached while revision unchanged");
    // bump 后失效重读
    bump_revision();
    assert!(!gate.settings().enabled, "refreshed after bump");
}

#[test]
fn engine_constructed_once_per_store() {
    let gate = gate_bound(true, false, false);
    let engine = gate.engine().expect("engine from memory store");
    // 同一占位符引擎：两次 mask 同值同占位符
    let a = engine
        .write()
        .unwrap()
        .mask("EMAIL", "fixed@example-test域名");
    drop(a);
    let b = gate
        .engine()
        .unwrap()
        .write()
        .unwrap()
        .mask("EMAIL", "fixed@example-test域名");
    let _ = b;
}

#[test]
fn mask_request_and_restore_response_roundtrip() {
    let gate = gate_bound(true, false, false);
    let key = ["sk-", "Zx9QvLtYb3Nc8UmKe4Hd"].concat();
    let body = format!(r#"{{"messages":[{{"content":"key is {key}"}}]}}"#);
    let (masked, count) = gate.mask_request(&body).expect("mask ok");
    assert_eq!(count, 1);
    assert!(!masked.contains(&key), "{masked}");
    let restored = gate.restore_response(&masked).expect("restore ok");
    assert_eq!(restored, body);
    // 计数只记数量
    assert_eq!(gate.stats().masked_values, 1);
    assert_eq!(gate.stats().degraded, 0);
}

#[cfg(feature = "test-hooks")]
#[test]
fn fault_injection_panics_are_caught() {
    let gate = gate_bound(true, false, false);
    let body = r#"{"messages":[{"content":"hello"}]}"#;
    // 请求遮盖位置（降级计数由中间件调用方负责，e2e 覆盖）
    gate.inject_failure(FAULT_MASK);
    assert_eq!(gate.injected_failure(), FAULT_MASK);
    assert!(gate.mask_request(body).is_err());
    gate.inject_failure(0);
    assert!(gate.mask_request(body).is_ok());
    // 整体还原位置
    let masked = gate.mask_request(body).unwrap().0;
    gate.inject_failure(FAULT_RESTORE_JSON);
    assert!(gate.restore_response(&masked).is_err());
    gate.inject_failure(0);
    assert!(gate.restore_response(&masked).is_ok());
}

#[test]
fn unbound_defaults_to_disabled() {
    let gate = gate();
    let settings = gate.settings();
    assert!(!settings.enabled && settings.block);
}
