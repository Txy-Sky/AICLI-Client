use super::*;
use crate::aicli::secrets::MemoryStore;
use rusqlite::types::Value as SqlValue;
use serde_json::json;
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const ACCESS: &str = "synthetic-system-access-canary";
const MODEL_KEY: &str = concat!("sk-", "c105-model-canary");
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

#[derive(Default)]
struct Remote {
    replies: Mutex<VecDeque<Result<Value, NewApiError>>>,
    credentials: Mutex<Vec<(String, String)>>,
    deleted: Mutex<Vec<i64>>,
    fail_delete: AtomicBool,
}

impl AccountRemote for Remote {
    fn verify<'a>(
        &'a self,
        access: &'a str,
        selected_line: &'a str,
    ) -> BoxFuture<'a, Result<Value, NewApiError>> {
        Box::pin(async move {
            self.credentials
                .lock()
                .unwrap()
                .push((access.to_owned(), selected_line.to_owned()));
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected verification")
        })
    }
    fn delete<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
        id: i64,
    ) -> BoxFuture<'a, Result<(), NewApiError>> {
        Box::pin(async move {
            self.deleted.lock().unwrap().push(id);
            if self.fail_delete.load(Ordering::SeqCst) {
                Err(NewApiError::TokenInvalid)
            } else {
                Ok(())
            }
        })
    }
}

#[derive(Default)]
struct Lifecycle {
    refreshes: AtomicUsize,
    restores: AtomicUsize,
    fail_restore: AtomicBool,
}

impl AccountLifecycle for Lifecycle {
    fn refresh(&self) {
        self.refreshes.fetch_add(1, Ordering::SeqCst);
    }
    fn release_all(&self) -> BoxFuture<'_, Result<(), AccountError>> {
        Box::pin(async move {
            self.restores.fetch_add(1, Ordering::SeqCst);
            if self.fail_restore.load(Ordering::SeqCst) {
                Err(AccountError::Restore)
            } else {
                Ok(())
            }
        })
    }
}

struct Fixture {
    temp: tempfile::TempDir,
    db: Arc<Database>,
    secrets: Arc<MemoryStore>,
    lifecycle: Arc<Lifecycle>,
    remote: Arc<Remote>,
    service: AccountService,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let conn = Connection::open(temp.path().join("account.db")).unwrap();
        Database::create_tables_on_conn(&conn).unwrap();
        let db = Arc::new(Database {
            conn: Mutex::new(conn),
            log_count_cache: Mutex::new(None),
        });
        let secrets = Arc::new(MemoryStore::default());
        let lifecycle = Arc::new(Lifecycle::default());
        let remote = Arc::new(Remote::default());
        let mut service =
            AccountService::new(db.clone(), secrets.clone(), lifecycle.clone()).unwrap();
        service.remote = remote.clone();
        Self {
            temp,
            db,
            secrets,
            lifecycle,
            remote,
            service,
        }
    }

    fn reply(&self, id: i64) {
        self.remote.replies.lock().unwrap().push_back(Ok(json!({"success":true,"data":{"id":id,"username":format!("fixture-{id}"),"group":"default"}})));
    }

    async fn bind(&self) {
        self.reply(7);
        self.service.bind(ACCESS, "main").await.unwrap();
    }

    fn seed_user_and_device_data(&self) {
        self.db.conn.lock().unwrap().execute_batch("INSERT INTO aicli_cache VALUES ('account','fingerprint',123,'{\"safe\":1}');
            INSERT INTO aicli_profile VALUES ('profile','fixture','user',NULL,1,'{}',123,123);
            INSERT INTO aicli_scope_binding VALUES ('global','','profile');
            UPDATE aicli_account SET active_profile_id='profile',privacy_enabled=1,last_sync_at=123;
            INSERT INTO aicli_managed_token VALUES (7,'hash-seven','[\"g-cheap\"]','aicli:seven','masked',1);
            INSERT INTO aicli_managed_token VALUES (8,'hash-eight','[\"g-mid\"]','user-owned','masked',0);
            INSERT INTO aicli_usage_daily VALUES ('2026-01-01',123,1,1,123);
            INSERT INTO aicli_local_token VALUES ('claude','aicli-local-synthetic',123);
            INSERT INTO aicli_line_health VALUES ('main',123,'healthy',1,200,NULL);
            INSERT INTO aicli_takeover VALUES ('claude','active','takeover-backup:claude',123);
            INSERT INTO proxy_request_logs (request_id,provider_id,app_type,model,latency_ms,status_code,created_at)
                VALUES ('preserved-log','provider','claude','model',1,200,123);").unwrap();
        self.secrets.set("token:7", MODEL_KEY).unwrap();
        self.secrets
            .set("token:8", concat!("sk-", "c105-reused-canary"))
            .unwrap();
        self.secrets
            .set("privacy-hmac-key", "synthetic-private-hmac")
            .unwrap();
        self.secrets
            .set("takeover-backup:claude", "synthetic-device-backup")
            .unwrap();
    }

    fn rows(&self, table: &str) -> Vec<Vec<SqlValue>> {
        let conn = self.db.conn.lock().unwrap();
        let mut statement = conn.prepare(&format!("SELECT * FROM {table}")).unwrap();
        let columns = statement.column_count();
        let rows = statement
            .query_map([], |row| (0..columns).map(|i| row.get(i)).collect())
            .unwrap();
        rows.collect::<Result<Vec<_>, _>>().unwrap()
    }

    fn snapshot(&self) -> BTreeMap<&'static str, Vec<Vec<SqlValue>>> {
        TABLES
            .into_iter()
            .map(|table| (table, self.rows(table)))
            .collect()
    }

    fn assert_no_credentials_in_database(&self) {
        for entry in std::fs::read_dir(self.temp.path()).unwrap() {
            let bytes = std::fs::read(entry.unwrap().path()).unwrap();
            for forbidden in [
                ACCESS,
                MODEL_KEY,
                "synthetic-replacement-access",
                concat!("sk-", "c105-reused-canary"),
            ] {
                assert!(
                    !bytes
                        .windows(forbidden.len())
                        .any(|window| window == forbidden.as_bytes()),
                    "credential reached SQLite"
                );
            }
        }
    }
}

#[tokio::test]
async fn rejects_api_keys_empty_tokens_and_unknown_lines_without_network_or_storage() {
    let fixture = Fixture::new();
    let before = fixture.snapshot();
    for access in [
        concat!("sk-", "synthetic-wrong-credential"),
        concat!(" \n sk-", "synthetic-wrong-credential \t"),
    ] {
        assert_eq!(
            fixture.service.bind(access, "main").await,
            Err(AccountError::WrongCredentialType)
        );
    }
    assert_eq!(
        fixture.service.bind(" \t ", "main").await,
        Err(AccountError::Api(NewApiError::InvalidInput))
    );
    assert_eq!(
        fixture.service.bind(ACCESS, "unknown").await,
        Err(AccountError::Api(NewApiError::UnknownLine))
    );
    assert!(fixture.remote.credentials.lock().unwrap().is_empty());
    assert_eq!(fixture.secrets.get("access-token").unwrap(), None);
    assert_eq!(fixture.snapshot(), before);
    assert_eq!(fixture.lifecycle.refreshes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn trims_validates_then_stores_only_public_account_and_schedules_refresh() {
    let fixture = Fixture::new();
    fixture.reply(7);
    let account = fixture
        .service
        .bind(&format!(" \n{ACCESS}\t"), "global")
        .await
        .unwrap();
    assert_eq!(account.user_id, 7);
    assert_eq!(account.selected_line_id, "global");
    assert_eq!(
        fixture.remote.credentials.lock().unwrap().as_slice(),
        &[(ACCESS.to_owned(), "global".to_owned())]
    );
    assert_eq!(
        fixture.secrets.get("access-token").unwrap().as_deref(),
        Some(ACCESS)
    );
    assert_eq!(fixture.service.account().unwrap(), Some(account));
    assert_eq!(fixture.service.state(), BindingState::Bound);
    assert_eq!(fixture.lifecycle.refreshes.load(Ordering::SeqCst), 1);
    assert!(fixture.service.require_bound().is_ok());
    fixture.assert_no_credentials_in_database();
    for forbidden in [ACCESS, MODEL_KEY] {
        assert!(!format!("{:?}", fixture.service).contains(forbidden));
    }
}

#[tokio::test]
async fn failed_or_malformed_verification_preserves_existing_binding_and_all_data() {
    let fixture = Fixture::new();
    fixture.bind().await;
    fixture.seed_user_and_device_data();
    let before = fixture.snapshot();
    for reply in [
        Err(NewApiError::TokenInvalid),
        Err(NewApiError::Network),
        Ok(json!({"success":true,"data":{"id":0}})),
    ] {
        fixture.remote.replies.lock().unwrap().push_back(reply);
        assert!(fixture
            .service
            .bind("synthetic-wrong-access", "apac")
            .await
            .is_err());
        assert_eq!(fixture.snapshot(), before);
        assert_eq!(
            fixture.secrets.get("access-token").unwrap().as_deref(),
            Some(ACCESS)
        );
        assert_eq!(fixture.service.state(), BindingState::Bound);
    }
    assert_eq!(fixture.lifecycle.refreshes.load(Ordering::SeqCst), 1);
    fixture.assert_no_credentials_in_database();
}

#[tokio::test]
async fn switching_user_clears_user_data_and_old_model_keys_but_preserves_device_backups_and_logs()
{
    let fixture = Fixture::new();
    fixture.bind().await;
    fixture.seed_user_and_device_data();
    let before = fixture.snapshot();
    fixture.reply(9);
    fixture
        .service
        .bind("synthetic-replacement-access", "apac")
        .await
        .unwrap();
    for table in [
        "aicli_cache",
        "aicli_profile",
        "aicli_scope_binding",
        "aicli_managed_token",
        "aicli_usage_daily",
    ] {
        assert!(fixture.rows(table).is_empty());
    }
    for table in [
        "aicli_local_token",
        "aicli_line_health",
        "aicli_takeover",
        "proxy_request_logs",
    ] {
        assert_eq!(fixture.rows(table), before[table]);
    }
    assert_eq!(fixture.service.account().unwrap().unwrap().user_id, 9);
    assert_eq!(fixture.secrets.get("token:7").unwrap(), None);
    assert_eq!(fixture.secrets.get("token:8").unwrap(), None);
    assert_eq!(
        fixture
            .secrets
            .get("takeover-backup:claude")
            .unwrap()
            .as_deref(),
        Some("synthetic-device-backup")
    );
    assert!(fixture.remote.deleted.lock().unwrap().is_empty());
    assert_eq!(fixture.lifecycle.restores.load(Ordering::SeqCst), 0);
    fixture.assert_no_credentials_in_database();
}

#[tokio::test]
async fn invalidation_keeps_cache_readable_and_rebinding_same_user_preserves_profiles_tokens_and_cache(
) {
    let fixture = Fixture::new();
    fixture.bind().await;
    fixture.seed_user_and_device_data();
    let before = fixture.snapshot();
    fixture
        .service
        .observe_api_error(fixture.service.generation(), NewApiError::Network);
    assert_eq!(fixture.service.state(), BindingState::Bound);
    fixture
        .service
        .observe_api_error(fixture.service.generation(), NewApiError::TokenInvalid);
    assert_eq!(fixture.service.state(), BindingState::NeedsRebind);
    assert_eq!(fixture.service.require_bound(), Err(AccountError::NotBound));
    assert_eq!(fixture.service.account().unwrap().unwrap().user_id, 7);
    assert_eq!(fixture.snapshot(), before);
    fixture.reply(7);
    fixture
        .service
        .bind("synthetic-replacement-access", "main")
        .await
        .unwrap();
    assert_eq!(fixture.snapshot(), before);
    assert_eq!(fixture.service.state(), BindingState::Bound);
    assert_eq!(
        fixture.secrets.get("token:7").unwrap().as_deref(),
        Some(MODEL_KEY)
    );
    assert_eq!(
        fixture.secrets.get("access-token").unwrap().as_deref(),
        Some("synthetic-replacement-access")
    );
    fixture.assert_no_credentials_in_database();
}

#[tokio::test]
async fn unbind_restores_takeovers_removes_credentials_and_account_tables_and_preserves_logs() {
    for delete_created in [false, true] {
        let fixture = Fixture::new();
        fixture.bind().await;
        fixture.seed_user_and_device_data();
        let logs = fixture.rows("proxy_request_logs");
        fixture.service.unbind(delete_created).await.unwrap();
        assert_eq!(fixture.lifecycle.restores.load(Ordering::SeqCst), 1);
        let deleted = fixture.remote.deleted.lock().unwrap().clone();
        assert_eq!(deleted, if delete_created { vec![7] } else { vec![] });
        for table in TABLES
            .into_iter()
            .filter(|table| *table != "proxy_request_logs")
        {
            assert!(fixture.rows(table).is_empty(), "{table}");
        }
        assert_eq!(fixture.rows("proxy_request_logs"), logs);
        for key in ["access-token", "token:7", "token:8"] {
            assert_eq!(fixture.secrets.get(key).unwrap(), None);
        }
        assert_eq!(
            fixture.secrets.get("privacy-hmac-key").unwrap().as_deref(),
            Some("synthetic-private-hmac")
        );
        assert_eq!(fixture.service.state(), BindingState::NotBound);
        fixture.assert_no_credentials_in_database();
    }
}

#[tokio::test]
async fn restoration_failure_preserves_binding_credentials_and_every_table() {
    let fixture = Fixture::new();
    fixture.bind().await;
    fixture.seed_user_and_device_data();
    let before = fixture.snapshot();
    fixture.lifecycle.fail_restore.store(true, Ordering::SeqCst);
    assert_eq!(
        fixture.service.unbind(true).await,
        Err(AccountError::Restore)
    );
    assert_eq!(fixture.snapshot(), before);
    assert_eq!(
        fixture.secrets.get("access-token").unwrap().as_deref(),
        Some(ACCESS)
    );
    assert!(fixture.remote.deleted.lock().unwrap().is_empty());
    assert_eq!(fixture.service.state(), BindingState::Bound);
}

#[tokio::test]
async fn remote_delete_failure_does_not_clear_local_account_data_or_reused_tokens() {
    let fixture = Fixture::new();
    fixture.bind().await;
    fixture.seed_user_and_device_data();
    let before = fixture.snapshot();
    fixture.remote.fail_delete.store(true, Ordering::SeqCst);
    assert_eq!(
        fixture.service.unbind(true).await,
        Err(AccountError::Api(NewApiError::TokenInvalid))
    );
    assert_eq!(fixture.snapshot(), before);
    assert_eq!(fixture.service.state(), BindingState::NeedsRebind);
    assert_eq!(fixture.remote.deleted.lock().unwrap().as_slice(), &[7]);
    assert_eq!(
        fixture.secrets.get("token:8").unwrap().as_deref(),
        Some(concat!("sk-", "c105-reused-canary"))
    );
}

#[tokio::test]
async fn reopening_uses_only_local_account_and_credential_presence() {
    let fixture = Fixture::new();
    assert_eq!(fixture.service.state(), BindingState::NotBound);
    fixture.bind().await;
    let reopened = AccountService::new(
        fixture.db.clone(),
        fixture.secrets.clone(),
        fixture.lifecycle.clone(),
    )
    .unwrap();
    assert_eq!(reopened.state(), BindingState::Bound);
    fixture.secrets.delete("access-token").unwrap();
    let reopened = AccountService::new(
        fixture.db.clone(),
        fixture.secrets.clone(),
        fixture.lifecycle.clone(),
    )
    .unwrap();
    assert_eq!(reopened.state(), BindingState::NeedsRebind);
    assert_eq!(reopened.require_bound(), Err(AccountError::NotBound));
    assert!(reopened.account().unwrap().is_some());
}

#[tokio::test]
async fn late_invalid_token_error_cannot_invalidate_replacement_binding_or_unbound_state() {
    let fixture = Fixture::new();
    fixture.bind().await;
    let old_generation = fixture.service.generation();
    fixture.reply(7);
    fixture
        .service
        .bind("synthetic-replacement-access", "main")
        .await
        .unwrap();
    let new_generation = fixture.service.generation();
    assert_ne!(old_generation, new_generation);
    fixture
        .service
        .observe_api_error(old_generation, NewApiError::TokenInvalid);
    assert_eq!(fixture.service.state(), BindingState::Bound);
    fixture
        .service
        .observe_api_error(new_generation, NewApiError::TokenInvalid);
    assert_eq!(fixture.service.state(), BindingState::NeedsRebind);
    fixture.service.unbind(false).await.unwrap();
    fixture
        .service
        .observe_api_error(new_generation, NewApiError::TokenInvalid);
    assert_eq!(fixture.service.state(), BindingState::NotBound);
    assert_eq!(fixture.secrets.get("access-token").unwrap(), None);
    assert!(fixture.remote.deleted.lock().unwrap().is_empty());
}

#[derive(Debug)]
struct FailOneWrite {
    inner: Arc<MemoryStore>,
    next: AtomicBool,
}

impl SecretStore for FailOneWrite {
    fn get(&self, account: &str) -> Result<Option<String>, SecretError> {
        self.inner.get(account)
    }
    fn set(&self, account: &str, value: &str) -> Result<(), SecretError> {
        if self.next.swap(false, Ordering::SeqCst) {
            Err(SecretError::Write)
        } else {
            self.inner.set(account, value)
        }
    }
    fn delete(&self, account: &str) -> Result<(), SecretError> {
        self.inner.delete(account)
    }
}

#[tokio::test]
async fn failed_credential_write_rolls_back_sqlite_and_restores_previously_removed_model_keys() {
    let mut fixture = Fixture::new();
    fixture.bind().await;
    fixture.seed_user_and_device_data();
    let before = fixture.snapshot();
    fixture.service.secrets = Arc::new(FailOneWrite {
        inner: fixture.secrets.clone(),
        next: AtomicBool::new(true),
    });
    fixture.reply(9);
    assert_eq!(
        fixture
            .service
            .bind("synthetic-replacement-access", "apac")
            .await,
        Err(AccountError::Secret(SecretError::Write))
    );
    assert_eq!(fixture.snapshot(), before);
    assert_eq!(fixture.service.state(), BindingState::Bound);
    assert_eq!(
        fixture.secrets.get("access-token").unwrap().as_deref(),
        Some(ACCESS)
    );
    assert_eq!(
        fixture.secrets.get("token:7").unwrap().as_deref(),
        Some(MODEL_KEY)
    );
    assert_eq!(
        fixture.secrets.get("token:8").unwrap().as_deref(),
        Some(concat!("sk-", "c105-reused-canary"))
    );
    assert_eq!(fixture.lifecycle.refreshes.load(Ordering::SeqCst), 1);
    fixture.assert_no_credentials_in_database();
}

#[tokio::test]
async fn failed_database_write_does_not_change_credentials_or_delete_existing_rows() {
    let fixture = Fixture::new();
    fixture.bind().await;
    fixture.seed_user_and_device_data();
    let before = fixture.snapshot();
    fixture
        .db
        .conn
        .lock()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_account BEFORE UPDATE ON aicli_account
        BEGIN SELECT RAISE(ABORT, 'synthetic database failure'); END;",
        )
        .unwrap();
    fixture.reply(9);
    assert_eq!(
        fixture
            .service
            .bind("synthetic-replacement-access", "apac")
            .await,
        Err(AccountError::Database)
    );
    assert_eq!(fixture.snapshot(), before);
    assert_eq!(
        fixture.secrets.get("access-token").unwrap().as_deref(),
        Some(ACCESS)
    );
    assert_eq!(
        fixture.secrets.get("token:7").unwrap().as_deref(),
        Some(MODEL_KEY)
    );
    assert_eq!(fixture.lifecycle.refreshes.load(Ordering::SeqCst), 1);
    fixture.assert_no_credentials_in_database();
}
