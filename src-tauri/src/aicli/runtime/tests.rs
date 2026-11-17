use super::*;
use crate::aicli::secrets::MemoryStore;
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct Remote {
    fail: AtomicBool,
    model: Mutex<String>,
    fetches: AtomicUsize,
    token_services: AtomicUsize,
    creates: AtomicUsize,
}
impl Default for Remote {
    fn default() -> Self {
        Self {
            fail: false.into(),
            model: Mutex::new("model-a".into()),
            fetches: 0.into(),
            token_services: 0.into(),
            creates: 0.into(),
        }
    }
}
fn snapshots(model: &str) -> (Value, Value, Value, Value) {
    (
        json!({"data":{"id":7}}),
        json!({"data":{"auto":{},"group-a":{"ratio":1}}}),
        json!({"data":[{"model_name":model,"enable_groups":["group-a"],"supported_endpoint_types":["openai"]}]}),
        json!({"data":{"groups":["group-a"],"max_count":10}}),
    )
}
struct Tokens;
impl TokenRemote for Tokens {
    fn list(&self) -> BoxFuture<'_, Result<Vec<Value>, NewApiError>> {
        Box::pin(async {
            Ok(vec![
                json!({"id":17,"name":"user-owned","status":1,"expired_time":-1,"unlimited_quota":true,"model_limits_enabled":false,"allow_ips":null,"group":"group-a"}),
            ])
        })
    }
    fn create<'a>(&'a self, _: &'a Value) -> BoxFuture<'a, Result<(), NewApiError>> {
        Box::pin(async { panic!("compatible token must be reused") })
    }
    fn keys<'a>(
        &'a self,
        ids: &'a [i64],
    ) -> BoxFuture<'a, Result<BTreeMap<i64, String>, NewApiError>> {
        Box::pin(async move {
            assert_eq!(ids, [17]);
            Ok(BTreeMap::from([(
                17,
                "synthetic-runtime-credential".into(),
            )]))
        })
    }
}
impl RuntimeRemote for Remote {
    fn fetch<'a>(
        &'a self,
        _: &'a NewApiClient,
    ) -> BoxFuture<'a, Result<RuntimeValues, NewApiError>> {
        Box::pin(async move {
            self.fetches.fetch_add(1, Ordering::SeqCst);
            if self.fail.load(Ordering::SeqCst) {
                Err(NewApiError::Network)
            } else {
                Ok(snapshots(&self.model.lock().unwrap()))
            }
        })
    }
    fn tokens(&self, _: NewApiClient) -> Arc<dyn TokenRemote> {
        self.token_services.fetch_add(1, Ordering::SeqCst);
        Arc::new(Tokens)
    }
}
fn fixture(
    cached: bool,
) -> (
    Arc<AicliRuntime>,
    Arc<Database>,
    Arc<MemoryStore>,
    Arc<Remote>,
) {
    let db = Arc::new(Database::memory().unwrap());
    let secrets = Arc::new(MemoryStore::default());
    let remote = Arc::new(Remote::default());
    db.conn.lock().unwrap().execute("INSERT INTO aicli_account(id,user_id,username,user_group) VALUES(1,7,'fixture','group-a')",[]).unwrap();
    secrets.set("access-token", "synthetic-access").unwrap();
    if cached {
        let (_, groups, pricing, auto) = snapshots("model-a");
        for (kind, value) in [
            ("groups", groups),
            ("pricing", pricing),
            ("auto_groups", auto),
        ] {
            db.conn.lock().unwrap().execute("INSERT INTO aicli_cache(kind,fingerprint,fetched_at,body_json) VALUES(?1,'cached',1,?2)",params![kind,value.to_string()]).unwrap();
        }
        db.conn.lock().unwrap().execute("INSERT INTO aicli_managed_token VALUES(17,?1,'[\"group-a\"]','user-owned','masked',0)",[groups_hash(&["group-a".into()])]).unwrap();
        secrets
            .set("token:17", "synthetic-cached-credential")
            .unwrap();
    }
    (
        AicliRuntime::with_remote(db.clone(), secrets.clone(), remote.clone()),
        db,
        secrets,
        remote,
    )
}

#[tokio::test]
async fn offline_start_has_cached_routes_and_credentials_without_any_network() {
    let (runtime, _, _, remote) = fixture(true);
    remote.fail.store(true, Ordering::SeqCst);
    assert_eq!(remote.fetches.load(Ordering::SeqCst), 0);
    let cached = runtime.snapshot();
    let profile = &cached.profiles["recommended"];
    assert_eq!(profile.compiled.routes[0].public_model, "model-a");
    assert_eq!(profile.token_ids["model-a"], 17);
    assert_eq!(
        runtime.credential(17).as_deref(),
        Some("synthetic-cached-credential")
    );
    assert!(runtime.refresh().await.is_err());
    assert!(Arc::ptr_eq(&cached, &runtime.snapshot()));
}

#[tokio::test]
async fn refresh_failure_keeps_entire_old_snapshot_and_secret_values() {
    let (runtime, _, _, remote) = fixture(false);
    runtime.refresh().await.unwrap();
    let old = runtime.snapshot();
    let key = runtime.credential(17);
    remote.fail.store(true, Ordering::SeqCst);
    assert!(runtime.refresh().await.is_err());
    assert!(Arc::ptr_eq(&old, &runtime.snapshot()));
    assert_eq!(key, runtime.credential(17));
    assert_eq!(old.profiles["recommended"].token_ids["model-a"], 17);
}

#[tokio::test]
async fn unbound_runtime_is_empty_and_does_not_fetch_or_create_tokens() {
    let (runtime, db, _, remote) = fixture(true);
    db.conn
        .lock()
        .unwrap()
        .execute("DELETE FROM aicli_account", [])
        .unwrap();
    runtime.refresh().await.unwrap();
    assert!(runtime.snapshot().profiles.is_empty());
    assert!(runtime.credential(17).is_none());
    assert_eq!(remote.fetches.load(Ordering::SeqCst), 0);
    let reopened = AicliRuntime::with_remote(db, runtime.store.clone(), remote.clone());
    assert!(reopened.snapshot().catalog.is_none());
    assert_eq!(remote.token_services.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn old_session_snapshot_survives_catalog_replacement() {
    let (runtime, _, _, remote) = fixture(true);
    let old = runtime.snapshot();
    runtime.pin("session".into(), "recommended".into(), old.clone());
    *remote.model.lock().unwrap() = "model-b".into();
    runtime.refresh().await.unwrap();
    let new = runtime.snapshot();
    assert!(!Arc::ptr_eq(&old, &new));
    assert!(new.catalog.as_ref().unwrap().models.contains_key("model-b"));
    let pin = runtime.session_pin("session").unwrap();
    assert!(Arc::ptr_eq(&old, &pin.snapshot));
    assert!(pin
        .snapshot
        .catalog
        .as_ref()
        .unwrap()
        .models
        .contains_key("model-a"));
    assert!(!pin
        .snapshot
        .catalog
        .as_ref()
        .unwrap()
        .models
        .contains_key("model-b"));
    assert_eq!(pin.profile_id, "recommended");
}

#[tokio::test]
async fn unchanged_fingerprint_preserves_arc_and_does_not_repeat_token_setup() {
    let (runtime, _, _, remote) = fixture(false);
    runtime.refresh().await.unwrap();
    let old = runtime.snapshot();
    runtime.refresh().await.unwrap();
    assert!(Arc::ptr_eq(&old, &runtime.snapshot()));
    assert_eq!(remote.token_services.load(Ordering::SeqCst), 1);
    assert_eq!(remote.creates.load(Ordering::SeqCst), 0);
}

#[test]
fn pins_are_bounded_lru_and_expire_after_twenty_four_hours() {
    let (runtime, _, _, _) = fixture(false);
    let snapshot = runtime.snapshot();
    for i in 0..SESSION_CAPACITY {
        runtime.pin(i.to_string(), "recommended".into(), snapshot.clone());
    }
    assert!(runtime.session_pin("0").is_some());
    runtime.pin("last".into(), "recommended".into(), snapshot.clone());
    assert!(runtime.session_pin("1").is_none());
    assert!(runtime.session_pin("0").is_some());
    assert_eq!(runtime.session_pins.lock().unwrap().len(), SESSION_CAPACITY);
    // Expiry is exercised through the injectable TTL: `Instant::now() - TTL`
    // overflows on hosts whose uptime is shorter than the TTL, because the
    // monotonic clock epoch is boot time.
    assert_eq!(SESSION_TTL, Duration::from_secs(24 * 60 * 60));
    *runtime.session_ttl.lock().unwrap() = Duration::from_secs(0);
    assert!(runtime.session_pin("0").is_none());
    *runtime.session_ttl.lock().unwrap() = SESSION_TTL;
    assert!(runtime.session_pin("last").is_some());
}

#[tokio::test]
async fn credential_loss_clears_routes_and_pins_even_with_persisted_account() {
    let (runtime, _, secrets, remote) = fixture(true);
    runtime.pin("session".into(), "recommended".into(), runtime.snapshot());
    secrets.delete("access-token").unwrap();
    runtime.refresh().await.unwrap();
    assert!(runtime.snapshot().profiles.is_empty());
    assert!(runtime.session_pin("session").is_none());
    assert!(runtime.credential(17).is_none());
    assert_eq!(remote.fetches.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn refreshed_public_caches_allow_offline_reopen_and_contain_no_credentials() {
    let (runtime, db, secrets, remote) = fixture(false);
    runtime.refresh().await.unwrap();
    remote.fail.store(true, Ordering::SeqCst);
    let reopened = AicliRuntime::with_remote(db.clone(), secrets, remote);
    assert_eq!(
        reopened.snapshot().profiles["recommended"].token_ids["model-a"],
        17
    );
    let conn = db.conn.lock().unwrap();
    let mut statement = conn.prepare("SELECT body_json FROM aicli_cache").unwrap();
    for body in statement.query_map([], |r| r.get::<_, String>(0)).unwrap() {
        let body = body.unwrap();
        assert!(!body.contains("synthetic-access"));
        assert!(!body.contains("synthetic-runtime-credential"));
    }
    assert!(!format!("{:?}", reopened.snapshot()).contains("synthetic"));
}

#[tokio::test]
async fn late_response_after_unbind_cannot_restore_old_account_routes() {
    struct Holding {
        started: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }
    impl RuntimeRemote for Holding {
        fn fetch<'a>(
            &'a self,
            _: &'a NewApiClient,
        ) -> BoxFuture<'a, Result<RuntimeValues, NewApiError>> {
            Box::pin(async move {
                self.started.notify_one();
                self.release.notified().await;
                Ok(snapshots("late-model"))
            })
        }
        fn tokens(&self, _: NewApiClient) -> Arc<dyn TokenRemote> {
            panic!("late account response must not reach token operations")
        }
    }
    let (_, db, secrets, _) = fixture(true);
    let remote = Arc::new(Holding {
        started: Default::default(),
        release: Default::default(),
    });
    let runtime = AicliRuntime::with_remote(db.clone(), secrets.clone(), remote.clone());
    let task = {
        let runtime = runtime.clone();
        tokio::spawn(async move { runtime.refresh().await })
    };
    remote.started.notified().await;
    db.conn
        .lock()
        .unwrap()
        .execute("DELETE FROM aicli_account", [])
        .unwrap();
    secrets.delete("access-token").unwrap();
    remote.release.notify_one();
    task.await.unwrap().unwrap();
    assert!(runtime.snapshot().profiles.is_empty());
    assert!(runtime.credential(17).is_none());
}

#[tokio::test]
async fn rebind_clears_previous_credentials_and_session_pins_before_fetching() {
    let (runtime, _, secrets, remote) = fixture(true);
    runtime.pin(
        "old-account".into(),
        "recommended".into(),
        runtime.snapshot(),
    );
    secrets.set("access-token", "synthetic-new-access").unwrap();
    // Binding clears cached account state in the account lifecycle. Keep that
    // same invariant in this test so a failed fetch cannot adopt old data.
    runtime
        .db
        .conn
        .lock()
        .unwrap()
        .execute("DELETE FROM aicli_cache", [])
        .unwrap();
    runtime
        .db
        .conn
        .lock()
        .unwrap()
        .execute("DELETE FROM aicli_managed_token", [])
        .unwrap();
    remote.fail.store(true, Ordering::SeqCst);
    assert!(runtime.refresh().await.is_err());
    assert!(runtime.session_pin("old-account").is_none());
    assert!(runtime.credential(17).is_none());
    assert!(runtime.snapshot().profiles.is_empty());
}

#[tokio::test]
async fn aicli_error_recovery_refreshes_credentials_once_without_fetching_catalog() {
    let (runtime, _, secrets, remote) = fixture(true);
    runtime.pin(
        "existing-session".into(),
        "recommended".into(),
        runtime.snapshot(),
    );
    runtime.recover_model_token(17).await.unwrap();
    assert_eq!(
        runtime.credential(17).as_deref(),
        Some(concat!("sk-", "synthetic-runtime-credential"))
    );
    assert_eq!(
        secrets.get("token:17").unwrap().as_deref(),
        Some(concat!("sk-", "synthetic-runtime-credential"))
    );
    assert_eq!(remote.fetches.load(Ordering::SeqCst), 0);
    assert_eq!(remote.token_services.load(Ordering::SeqCst), 1);
    assert!(matches!(
        runtime.recover_model_token(17).await,
        Err(RuntimeError::Token(TokenError::RecoveryExhausted))
    ));
    assert_eq!(remote.token_services.load(Ordering::SeqCst), 1);
}
