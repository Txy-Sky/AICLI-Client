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
    // data: URL 与文件字段
    assert!(contains_unscannable_media(
        r#"{"content":[{"type":"image","source":{"data":"data:image/png;base64,iVBOR"}}]}"#
    ));
    assert!(contains_unscannable_media(r#"{"file_data":"AAAA"}"#));
    // Claude source{type:"base64",data}：image 与 document 同形
    assert!(contains_unscannable_media(
        r#"{"messages":[{"content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"iVBOR"}}]}]}"#
    ));
    assert!(contains_unscannable_media(
        r#"{"messages":[{"content":[{"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"ZG9j"}}]}]}"#
    ));
    // Gemini inlineData / inline_data
    assert!(contains_unscannable_media(
        r#"{"contents":[{"parts":[{"inlineData":{"mimeType":"image/png","data":"iVBOR"}}]}]}"#
    ));
    assert!(contains_unscannable_media(
        r#"{"contents":[{"parts":[{"inline_data":{"mime_type":"image/png","data":"iVBOR"}}]}]}"#
    ));
    // 纯文本与无媒体 URL 形态
    assert!(!contains_unscannable_media(
        r#"{"content":[{"type":"text","text":"plain"}]}"#
    ));
    assert!(!contains_unscannable_media(
        r#"{"messages":[{"content":[{"type":"image","source":{"type":"url","url":"https://x/a.png"}}]}]}"#
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

/// 查库出错（连接互斥量中毒）时：有上一次成功快照就保留，隐私不会被
/// 悄悄关掉；从未成功过则 fail-closed 按开启 + block。
#[test]
fn refresh_error_keeps_last_good_settings() {
    let gate = gate_bound(true, true, false);
    assert!(gate.settings().enabled, "first read succeeds");
    // 中毒 db.conn：此后一切查库都出错
    let db = gate.db.clone();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = db.conn.lock().unwrap();
        panic!("poison the connection mutex");
    }));
    // bump 触发重读 → 出错 → 保留上一次快照（仍开启 + block）
    bump_revision();
    let settings = gate.settings();
    assert!(settings.enabled, "db error must not disable privacy");
    assert!(settings.block);
}

#[test]
fn refresh_error_without_snapshot_fails_closed() {
    // 构造后从未读过设置，直接中毒：fail-closed 开启 + block
    let gate = gate_bound(false, false, false);
    let db = gate.db.clone();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = db.conn.lock().unwrap();
        panic!("poison the connection mutex");
    }));
    let settings = gate.settings();
    assert!(settings.enabled, "no snapshot: fail closed");
    assert!(settings.block);
}

/// 未绑定（无账户行）= 关闭。
#[test]
fn unbound_row_means_disabled() {
    let db = Arc::new(Database::memory().unwrap());
    crate::aicli::seed::init(&db).unwrap();
    // seed 不一定建账户行；删掉确保没有
    db.conn
        .lock()
        .unwrap()
        .execute("DELETE FROM aicli_account", [])
        .unwrap();
    let store: Arc<dyn SecretStore> = Arc::new(MemoryStore::default());
    let gate = PrivacyGate::new(db, store);
    assert!(!gate.settings().enabled);
}
