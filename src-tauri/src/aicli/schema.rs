//! AICLI-owned local SQLite tables. Upstream credentials are never stored here.

use crate::error::AppError;
use rusqlite::Connection;

pub(crate) const REQUEST_LOG_COLUMNS: [(&str, &str); 6] = [
    ("aicli_line_id", "TEXT"),
    ("aicli_profile_id", "TEXT"),
    ("upstream_request_id", "TEXT"),
    ("server_group", "TEXT"),
    ("billed_quota", "INTEGER"),
    ("billed_at", "INTEGER"),
];

pub(crate) fn create_tables(conn: &Connection) -> Result<(), AppError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS aicli_account (
            id INTEGER PRIMARY KEY CHECK(id=1), user_id INTEGER, username TEXT, user_group TEXT,
            selected_line_id TEXT NOT NULL DEFAULT 'main', active_profile_id TEXT,
            privacy_enabled INTEGER NOT NULL DEFAULT 0,
            privacy_failure_mode TEXT NOT NULL DEFAULT 'block',
            allow_unscanned_media INTEGER NOT NULL DEFAULT 0,
            privacy_detect_cjk_names INTEGER NOT NULL DEFAULT 0, last_sync_at INTEGER
        );
        CREATE TABLE IF NOT EXISTS aicli_cache (
            kind TEXT PRIMARY KEY, fingerprint TEXT, fetched_at INTEGER NOT NULL,
            body_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS aicli_profile (
            id TEXT PRIMARY KEY, name TEXT NOT NULL, source TEXT NOT NULL, preset_id TEXT,
            revision INTEGER NOT NULL, body_json TEXT NOT NULL,
            created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS aicli_scope_binding (
            scope TEXT NOT NULL, scope_key TEXT NOT NULL, profile_id TEXT NOT NULL,
            PRIMARY KEY(scope, scope_key)
        );
        CREATE TABLE IF NOT EXISTS aicli_managed_token (
            token_id INTEGER PRIMARY KEY, groups_hash TEXT NOT NULL UNIQUE,
            groups_json TEXT NOT NULL, name TEXT NOT NULL, masked TEXT NOT NULL,
            created_by_client INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS aicli_usage_daily (
            day TEXT PRIMARY KEY, quota INTEGER NOT NULL, requests INTEGER NOT NULL,
            is_final INTEGER NOT NULL, fetched_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS aicli_local_token (
            app_type TEXT PRIMARY KEY, token TEXT NOT NULL, created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS aicli_line_health (
            line_id TEXT NOT NULL, checked_at INTEGER NOT NULL, status TEXT NOT NULL,
            latency_ms INTEGER, http_status INTEGER, error_code TEXT,
            PRIMARY KEY(line_id, checked_at)
        );
        CREATE TABLE IF NOT EXISTS aicli_takeover (
            app_type TEXT PRIMARY KEY, state TEXT NOT NULL, backup_ref TEXT,
            taken_over_at INTEGER
        );",
    )
    .map_err(|error| AppError::Database(error.to_string()))?;
    // E-08：privacy_detect_cjk_names 随 v25 之后的功能加入；CREATE TABLE
    // IF NOT EXISTS 不会给已存在的表补列，幂等 ALTER（沿用 REQUEST_LOG_COLUMNS
    // 的做法），不提升 SCHEMA_VERSION。
    ensure_columns(conn)
}

/// 幂等补列（已存在的旧库升级；新库 CREATE TABLE 已带上）。
pub(crate) fn ensure_columns(conn: &Connection) -> Result<(), AppError> {
    if crate::database::Database::has_column(conn, "aicli_account", "privacy_detect_cjk_names")? {
        return Ok(());
    }
    if !crate::database::Database::table_exists(conn, "aicli_account")? {
        return Ok(()); // 表还不存在（迁移稍后创建）
    }
    conn.execute(
        "ALTER TABLE aicli_account ADD COLUMN privacy_detect_cjk_names INTEGER NOT NULL DEFAULT 0",
        [],
    )
    .map_err(|error| AppError::Database(error.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{Database, SCHEMA_VERSION};
    use rusqlite::types::Value;

    const TABLES: [&str; 10] = [
        "aicli_account",
        "aicli_cache",
        "aicli_profile",
        "aicli_scope_binding",
        "aicli_managed_token",
        "aicli_usage_daily",
        "aicli_local_token",
        "aicli_line_health",
        "aicli_takeover",
        "proxy_request_logs",
    ];

    // Committed v24 log schema, with every preexisting column and one historical row.
    fn v24() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE proxy_request_logs (
                request_id TEXT PRIMARY KEY, provider_id TEXT NOT NULL, app_type TEXT NOT NULL, model TEXT NOT NULL,
                request_model TEXT, pricing_model TEXT,
                input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER NOT NULL DEFAULT 0, cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
                input_token_semantics INTEGER NOT NULL DEFAULT 0,
                input_cost_usd TEXT NOT NULL DEFAULT '0', output_cost_usd TEXT NOT NULL DEFAULT '0',
                cache_read_cost_usd TEXT NOT NULL DEFAULT '0', cache_creation_cost_usd TEXT NOT NULL DEFAULT '0',
                total_cost_usd TEXT NOT NULL DEFAULT '0', latency_ms INTEGER NOT NULL, first_token_ms INTEGER,
                duration_ms INTEGER, status_code INTEGER NOT NULL, error_message TEXT, session_id TEXT,
                provider_type TEXT, is_streaming INTEGER NOT NULL DEFAULT 0,
                cost_multiplier TEXT NOT NULL DEFAULT '1.0', created_at INTEGER NOT NULL,
                data_source TEXT NOT NULL DEFAULT 'proxy'
             );
             INSERT INTO proxy_request_logs
                (request_id, provider_id, app_type, model, input_tokens, output_tokens,
                 total_cost_usd, latency_ms, status_code, session_id, created_at)
                VALUES ('existing-request', 'existing-provider', 'claude', 'existing-model',
                        123, 456, '0.125', 23, 200, 'existing-session', 123456);
             CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT);
             INSERT INTO settings VALUES ('existing-setting', 'preserve-exact-value');
             PRAGMA user_version=24;"
        ).unwrap();
        conn
    }

    fn rows(conn: &Connection, sql: &str) -> Vec<Vec<Value>> {
        let mut statement = conn.prepare(sql).unwrap();
        let count = statement.column_count();
        let result = statement
            .query_map([], |row| (0..count).map(|index| row.get(index)).collect())
            .unwrap();
        result.collect::<Result<Vec<_>, _>>().unwrap()
    }

    #[test]
    fn v24_migration_preserves_every_existing_field_and_is_repeatable() {
        let conn = v24();
        let before_log = rows(&conn, "SELECT * FROM proxy_request_logs");
        let before_settings = rows(&conn, "SELECT * FROM settings");
        Database::apply_schema_migrations_on_conn(&conn).unwrap();
        assert_eq!(Database::get_user_version(&conn).unwrap(), 25);
        let after = rows(&conn, "SELECT * FROM proxy_request_logs");
        assert_eq!(&after[0][..before_log[0].len()], before_log[0]);
        assert_eq!(&after[0][before_log[0].len()..], vec![Value::Null; 6]);
        assert_eq!(rows(&conn, "SELECT * FROM settings"), before_settings);
        conn.execute(
            "INSERT INTO aicli_account (id, username) VALUES (1, 'test-account')",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO aicli_managed_token VALUES (7, 'hash', '[\"g-mid\"]', 'aicli:test', 'masked', 1)", []).unwrap();
        let before_account = rows(&conn, "SELECT * FROM aicli_account");
        let before_tokens = rows(&conn, "SELECT * FROM aicli_managed_token");
        Database::apply_schema_migrations_on_conn(&conn).unwrap();
        create_tables(&conn).unwrap();
        assert_eq!(rows(&conn, "SELECT * FROM proxy_request_logs"), after);
        // E-08 幂等补列：privacy_detect_cjk_names 存在且为默认 0，
        // 原有行重复迁移后不变（列在 CREATE TABLE 或幂等 ALTER 中带上）
        let after_account = rows(&conn, "SELECT * FROM aicli_account");
        assert_eq!(after_account, before_account);
        let detect: i64 = conn
            .query_row(
                "SELECT privacy_detect_cjk_names FROM aicli_account WHERE id=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(detect, 0);
        assert_eq!(
            rows(&conn, "SELECT * FROM aicli_managed_token"),
            before_tokens
        );
    }

    #[test]
    fn fresh_and_migrated_tables_have_identical_columns_and_indexes() {
        let migrated = v24();
        Database::apply_schema_migrations_on_conn(&migrated).unwrap();
        let fresh = Connection::open_in_memory().unwrap();
        Database::create_tables_on_conn(&fresh).unwrap();
        Database::apply_schema_migrations_on_conn(&fresh).unwrap();
        assert_eq!(Database::get_user_version(&fresh).unwrap(), SCHEMA_VERSION);
        for table in TABLES {
            let sql = format!("PRAGMA table_info('{table}')");
            assert_eq!(rows(&fresh, &sql), rows(&migrated, &sql), "{table}");
            // Existing proxy log secondary indexes are outside this migration's scope.
            if table != "proxy_request_logs" {
                let sql = format!("PRAGMA index_list('{table}')");
                assert_eq!(rows(&fresh, &sql), rows(&migrated, &sql), "{table}");
            }
        }
    }

    #[test]
    fn account_defaults_and_singleton_constraint_are_enforced() {
        let conn = v24();
        Database::apply_schema_migrations_on_conn(&conn).unwrap();
        conn.execute("INSERT INTO aicli_account(id) VALUES (1)", [])
            .unwrap();
        let defaults: (String, i64, String, i64) = conn.query_row(
            "SELECT selected_line_id, privacy_enabled, privacy_failure_mode, allow_unscanned_media FROM aicli_account",
            [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).unwrap();
        assert_eq!(defaults, ("main".into(), 0, "block".into(), 0));
        assert!(conn
            .execute("INSERT INTO aicli_account(id) VALUES (2)", [])
            .is_err());
        assert!(conn
            .execute("INSERT INTO aicli_account(id) VALUES (1)", [])
            .is_err());
    }

    #[test]
    fn group_hash_and_composite_primary_keys_are_enforced() {
        let conn = v24();
        Database::apply_schema_migrations_on_conn(&conn).unwrap();
        conn.execute(
            "INSERT INTO aicli_managed_token VALUES (1, 'same', '[]', 'aicli:test', 'masked', 1)",
            [],
        )
        .unwrap();
        assert!(conn.execute("INSERT INTO aicli_managed_token VALUES (2, 'same', '[]', 'aicli:test', 'masked', 1)", []).is_err());
        conn.execute(
            "INSERT INTO aicli_scope_binding VALUES ('app', 'claude', 'p1')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO aicli_scope_binding VALUES ('app', 'codex', 'p1')",
            [],
        )
        .unwrap();
        assert!(conn
            .execute(
                "INSERT INTO aicli_scope_binding VALUES ('app', 'claude', 'p2')",
                []
            )
            .is_err());
        conn.execute(
            "INSERT INTO aicli_line_health VALUES ('main', 1, 'ok', 10, 200, NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO aicli_line_health VALUES ('main', 2, 'ok', 10, 200, NULL)",
            [],
        )
        .unwrap();
        assert!(conn
            .execute(
                "INSERT INTO aicli_line_health VALUES ('main', 1, 'ok', 10, 200, NULL)",
                []
            )
            .is_err());
    }

    #[test]
    fn failed_v24_migration_rolls_back_schema_and_version() {
        let conn = v24();
        // A conflicting index forces failure after earlier DDL has executed.
        conn.execute("CREATE INDEX aicli_takeover ON settings(key)", [])
            .unwrap();
        assert!(Database::apply_schema_migrations_on_conn(&conn).is_err());
        assert_eq!(Database::get_user_version(&conn).unwrap(), 24);
        assert!(rows(
            &conn,
            "SELECT name FROM sqlite_master WHERE name='aicli_account'"
        )
        .is_empty());
        let columns = rows(&conn, "PRAGMA table_info('proxy_request_logs')");
        assert!(!columns
            .iter()
            .any(|row| row[1] == Value::Text("billed_quota".into())));
        assert_eq!(
            rows(&conn, "SELECT request_id FROM proxy_request_logs"),
            vec![vec![Value::Text("existing-request".into())]]
        );
    }
}
