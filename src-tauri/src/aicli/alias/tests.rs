use std::sync::Mutex;

use rusqlite::Connection;

use super::*;

fn database() -> (tempfile::TempDir, Database) {
    let temp = tempfile::tempdir().unwrap();
    let conn = Connection::open(temp.path().join("alias.db")).unwrap();
    Database::create_tables_on_conn(&conn).unwrap();
    let db = Database {
        conn: Mutex::new(conn),
        log_count_cache: Mutex::new(None),
    };
    (temp, db)
}

#[test]
fn claude_code_alias_round_trips_even_for_claude_named_models() {
    let app = AppType::Claude;
    for model in ["gpt-5.5", "claude-sonnet-5.5[1M]"] {
        let encoded = encode(&app, model, None).unwrap();
        assert_eq!(encoded, format!("aicli-claude-{model}"));
        assert_eq!(
            decode(&app, &encoded, None).unwrap().as_deref(),
            Some(model)
        );
    }
    assert_eq!(decode(&app, "plain-model", None).unwrap(), None);
}

#[test]
fn desktop_alias_is_persisted_unique_and_uses_the_existing_filter() {
    let (_temp, db) = database();
    let app = AppType::ClaudeDesktop;
    let first = encode(&app, "deepseek-v4-pro", Some(&db)).unwrap();
    let repeated = encode(&app, "deepseek-v4-pro", Some(&db)).unwrap();
    let second = encode(&app, "kimi-k3", Some(&db)).unwrap();

    assert_eq!(first, repeated);
    assert_ne!(first, second);
    assert!(first.starts_with(DESKTOP_PREFIX));
    assert_eq!(first.len(), DESKTOP_PREFIX.len() + 6);
    assert!(first[DESKTOP_PREFIX.len()..]
        .bytes()
        .all(|byte| byte.is_ascii_digit()));
    assert!(crate::mode::stack::desktop_keeps_model_id(&first));
    assert_eq!(crate::mode::stack::desktop_model_identity(&first), first);
    assert_eq!(
        decode(&app, &first, Some(&db)).unwrap().as_deref(),
        Some("deepseek-v4-pro")
    );
    assert_eq!(
        decode(&app, "claude-fable-aicli-123456", Some(&db)).unwrap(),
        None
    );

    let (_other_temp, reopened) = database();
    {
        let conn = db.conn.lock().unwrap();
        let body: String = conn
            .query_row(
                "SELECT body_json FROM aicli_cache WHERE kind='desktop_alias'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        reopened.conn.lock().unwrap().execute(
            "INSERT INTO aicli_cache(kind,fingerprint,fetched_at,body_json) VALUES('desktop_alias',NULL,1,?1)",
            [body],
        ).unwrap();
    }
    assert_eq!(
        decode(&app, &first, Some(&reopened)).unwrap().as_deref(),
        Some("deepseek-v4-pro")
    );
}

#[test]
fn default_model_aliases_use_role_names_and_fall_back_to_first_model() {
    let app = AppType::Claude;
    let models = vec![
        "alpha-model".to_string(),
        "Vendor Haiku".to_string(),
        "vendor-sonnet-v2".to_string(),
        "vendor-OPUS-v1".to_string(),
    ];
    let aliases = default_aliases(&app, &models, None).unwrap();
    assert_eq!(
        aliases["ANTHROPIC_DEFAULT_HAIKU_MODEL"],
        "aicli-claude-Vendor Haiku"
    );
    assert_eq!(
        aliases["ANTHROPIC_DEFAULT_SONNET_MODEL"],
        "aicli-claude-vendor-sonnet-v2"
    );
    assert_eq!(
        aliases["ANTHROPIC_DEFAULT_OPUS_MODEL"],
        "aicli-claude-vendor-OPUS-v1"
    );
    assert_eq!(
        aliases["ANTHROPIC_DEFAULT_FABLE_MODEL"],
        "aicli-claude-alpha-model"
    );
}

#[test]
fn other_clients_keep_models_unencoded_and_do_not_decode_aliases() {
    let app = AppType::Codex;
    assert_eq!(encode(&app, "gpt-5.5", None).unwrap(), "gpt-5.5");
    assert_eq!(decode(&app, "aicli-claude-gpt-5.5", None).unwrap(), None);
}

#[test]
fn desktop_date_suffix_identity_rule_is_shared() {
    assert_eq!(
        crate::mode::stack::desktop_model_identity("claude-fable-aicli-123456"),
        "claude-fable-aicli-123456"
    );
    assert_eq!(
        crate::mode::stack::desktop_model_identity("claude-fable-aicli-123456-20260609"),
        "claude-fable-aicli-123456"
    );
}
