use super::*;
use crate::aicli::secrets::{MemoryStore, SecretStore};
use rusqlite::Connection;
use serde_json::json;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::Notify;

const ACCESS: &str = "synthetic-system-access-canary";
const SECOND_ACCESS: &str = "synthetic-system-access-canary-2";
const MODEL_KEY: &str = concat!("sk-", "c311-model-canary");

#[derive(Default)]
struct NoEvents;

impl CacheEvents for NoEvents {
    fn data_updated(&self, _: &str) {}
}

struct AccountFake {
    replies: Mutex<VecDeque<Result<Value, NewApiError>>>,
    deletions: AtomicUsize,
}

impl Default for AccountFake {
    fn default() -> Self {
        Self {
            replies: Mutex::new(VecDeque::new()),
            deletions: AtomicUsize::new(0),
        }
    }
}

impl AccountRemote for AccountFake {
    fn verify<'a>(&'a self, _: &'a str, _: &'a str) -> BoxFuture<'a, Result<Value, NewApiError>> {
        Box::pin(async move {
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected account verification")
        })
    }
    fn delete<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
        _: i64,
    ) -> BoxFuture<'a, Result<(), NewApiError>> {
        Box::pin(async move {
            self.deletions.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}

/// Scripted cache remote: default replies are inert JSON; `hold_next` parks one
/// fetch until the test opens the gate so in-flight rebinding races are real.
struct ScriptedRemote {
    queue: Mutex<VecDeque<Result<Value, NewApiError>>>,
    hold_next: AtomicBool,
    gate: Notify,
    fetches: AtomicUsize,
}

impl Default for ScriptedRemote {
    fn default() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            hold_next: AtomicBool::new(false),
            gate: Notify::new(),
            fetches: AtomicUsize::new(0),
        }
    }
}

impl CacheRemote for ScriptedRemote {
    fn fetch<'a>(&'a self, kind: &'a str) -> BoxFuture<'a, Result<Value, NewApiError>> {
        Box::pin(async move {
            self.fetches.fetch_add(1, Ordering::SeqCst);
            // Take the reply before parking so a held fetch owns its own result
            // and later fetches cannot consume it.
            let reply = self.queue.lock().unwrap().pop_front().unwrap_or_else(|| {
                // Catalog-shape-valid defaults so reads never break parsing.
                match kind {
                    "pricing" => Ok(json!({"data":[]})),
                    "account" => {
                        Ok(json!({"data":{"id":1,"username":"fixture","group":"default"}}))
                    }
                    _ => Ok(json!({"data":{}})),
                }
            });
            if self.hold_next.swap(false, Ordering::SeqCst) {
                self.gate.notified().await;
            }
            reply
        })
    }
    fn fetch_usage<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Value, NewApiError>> {
        Box::pin(async move { Ok(json!({"data":{"quota":1,"requests":1}})) })
    }
}

struct Fixture {
    services: Arc<AicliServices>,
    db: Arc<Database>,
    secrets: Arc<MemoryStore>,
    _remote: Arc<ScriptedRemote>,
    account_remote: Arc<AccountFake>,
    built: Arc<Mutex<Vec<(i64, String, String)>>>,
    caches: Arc<Mutex<Vec<Arc<AicliCache>>>>,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let conn = Connection::open(temp.path().join("aicli-commands.db")).unwrap();
        Database::create_tables_on_conn(&conn).unwrap();
        let db = Arc::new(Database {
            conn: Mutex::new(conn),
            log_count_cache: Mutex::new(None),
        });
        let secrets = Arc::new(MemoryStore::default());
        let app_state = AppState::new(db.clone());
        let remote = Arc::new(ScriptedRemote::default());
        let account_remote = Arc::new(AccountFake::default());
        let built: Arc<Mutex<Vec<(i64, String, String)>>> = Arc::new(Mutex::new(Vec::new()));
        let caches: Arc<Mutex<Vec<Arc<AicliCache>>>> = Arc::new(Mutex::new(Vec::new()));
        let builder: CacheBuilder = {
            let remote = remote.clone();
            let built = built.clone();
            let caches = caches.clone();
            Arc::new(move |binding| {
                built.lock().unwrap().push((
                    binding.user_id,
                    binding.line.clone(),
                    binding.access.clone(),
                ));
                let mut cache =
                    AicliCache::with_remote(binding.db, remote.clone(), binding.events.clone());
                cache.install_guard(
                    binding.user_id,
                    &binding.access,
                    &binding.line,
                    binding.secrets.clone(),
                );
                let cache = Arc::new(cache);
                caches.lock().unwrap().push(cache.clone());
                Ok(cache)
            })
        };
        let probe: ProbeFn = Arc::new(|_| {
            let result: Result<(u16, Value), NewApiError> = Err(NewApiError::Network);
            Box::pin(async move { result })
        });
        let services = AicliServices::for_test(
            app_state,
            secrets.clone(),
            Arc::new(NoEvents),
            builder,
            probe,
            account_remote.clone(),
        );
        Self {
            services,
            db,
            secrets,
            _remote: remote,
            account_remote,
            built,
            caches,
        }
    }

    fn seed_bound(&self, user_id: i64, access: &str) {
        self.secrets.set("access-token", access).unwrap();
        self.db
            .conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO aicli_account(id,user_id,username,user_group,selected_line_id)
                 VALUES(1,?1,'fixture','default','main')
                 ON CONFLICT(id) DO UPDATE SET user_id=excluded.user_id,
                 selected_line_id=excluded.selected_line_id",
                [user_id],
            )
            .unwrap();
    }
}

/// Acceptance: serialized command results carry no `sk-` plaintext and no test
/// token value; masked previews (`sk-…`) from C1-09 are the only sk- strings.
fn assert_no_secret_material(output: &str) {
    assert!(!output.contains(ACCESS), "access token leaked: {output}");
    assert!(!output.contains(MODEL_KEY), "model key leaked: {output}");
    let mut offset = 0;
    while let Some(found) = output[offset..].find("sk-") {
        let at = offset + found;
        assert!(
            output[at + 3..].starts_with('…'),
            "unmasked sk- value leaked at byte {at}: {output}"
        );
        offset = at + 3;
    }
}

#[test]
fn backoff_follows_30_60_120_capped_at_600() {
    assert_eq!(backoff_seconds(0), 0);
    assert_eq!(backoff_seconds(1), 30);
    assert_eq!(backoff_seconds(2), 60);
    assert_eq!(backoff_seconds(3), 120);
    assert_eq!(backoff_seconds(4), 240);
    assert_eq!(backoff_seconds(5), 480);
    assert_eq!(backoff_seconds(6), 600);
    assert_eq!(backoff_seconds(40), 600);
}

#[test]
fn rebuild_reuses_cache_until_the_binding_changes() {
    let fixture = Fixture::new();
    fixture.seed_bound(7, ACCESS);
    fixture.services.rebuild().unwrap();
    fixture.services.rebuild().unwrap();
    assert_eq!(fixture.built.lock().unwrap().len(), 1);

    fixture.seed_bound(7, SECOND_ACCESS);
    fixture.services.rebuild().unwrap();
    let built = fixture.built.lock().unwrap();
    assert_eq!(built.len(), 2);
    assert_eq!(built[1], (7, "main".to_owned(), SECOND_ACCESS.to_owned()));

    fixture
        .db
        .conn
        .lock()
        .unwrap()
        .execute("DELETE FROM aicli_account", [])
        .unwrap();
    fixture.secrets.delete("access-token").unwrap();
    fixture.services.rebuild().unwrap();
    assert!(fixture.services.inner.lock().unwrap().cache.is_none());
}

#[tokio::test]
async fn stale_refresh_after_rebinding_drops_old_account_writes() {
    let fixture = Fixture::new();
    fixture.seed_bound(7, ACCESS);
    fixture.services.rebuild().unwrap();
    let old_cache = fixture.caches.lock().unwrap()[0].clone();

    // A refresh under the old binding fetches old-account data and stalls.
    fixture._remote.hold_next.store(true, Ordering::SeqCst);
    fixture
        ._remote
        .queue
        .lock()
        .unwrap()
        .push_back(Ok(json!({"old":"account-7"})));
    let _ = old_cache.get("groups");
    while fixture._remote.fetches.load(Ordering::SeqCst) == 0 {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // Rebind to another account; the aggregate swaps to a guarded new cache.
    fixture
        .db
        .conn
        .lock()
        .unwrap()
        .execute("UPDATE aicli_account SET user_id=8 WHERE id=1", [])
        .unwrap();
    fixture.secrets.set("access-token", SECOND_ACCESS).unwrap();
    fixture.services.rebuild().unwrap();
    let new_cache = fixture
        .services
        .inner
        .lock()
        .unwrap()
        .cache
        .clone()
        .unwrap();
    let _ = new_cache.get("groups");
    new_cache.wait_for_idle().await;

    // Releasing the stalled old fetch must not overwrite the new account data.
    fixture._remote.gate.notify_one();
    old_cache.wait_for_idle().await;
    let body = fixture
        .db
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT body_json FROM aicli_cache WHERE kind='groups'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    assert!(
        !body.contains("account-7"),
        "a stale refresh leaked another account's data: {body}"
    );
}

#[tokio::test]
async fn bind_rebuilds_the_aggregate_and_unbind_clears_it() {
    let fixture = Fixture::new();
    fixture.account_remote.replies.lock().unwrap().push_back(Ok(
        json!({"success":true,"data":{"id":8,"username":"fixture-8","group":"default"}}),
    ));
    fixture
        .services
        .bind(SECOND_ACCESS.to_owned(), "main".to_owned())
        .await
        .unwrap();
    assert_eq!(
        fixture.built.lock().unwrap().last().unwrap(),
        &(8, "main".to_owned(), SECOND_ACCESS.to_owned())
    );
    let cache = fixture
        .services
        .inner
        .lock()
        .unwrap()
        .cache
        .clone()
        .unwrap();
    cache.wait_for_idle().await;
    for kind in ACCOUNT_KINDS {
        assert!(cache.read(kind).unwrap().is_some(), "missing {kind}");
    }

    fixture.services.unbind(false).await.unwrap();
    assert!(fixture.services.inner.lock().unwrap().cache.is_none());
    assert!(fixture.secrets.get("access-token").unwrap().is_none());
    let rows: i64 = fixture
        .db
        .conn
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM aicli_account", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0);
    assert_eq!(
        fixture.services.get_account().unwrap().state,
        BindingState::NotBound
    );
}

#[tokio::test]
async fn serialized_outputs_carry_no_token_material() {
    let fixture = Fixture::new();
    fixture.seed_bound(7, ACCESS);
    fixture.secrets.set("token:900", MODEL_KEY).unwrap();
    {
        let conn = fixture.db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO aicli_managed_token VALUES(900,'hash','[\"g\"]','fixture','sk-…ue-5',1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO aicli_profile(id,name,source,preset_id,revision,body_json,created_at,updated_at)
             VALUES('p1','P','user',NULL,1,'{\"selected_groups\":[],\"selection_mode\":\"fixed_priority\",
             \"model_bindings\":[],\"resolved_conflicts\":[],\"line_policy\":\"inherit\",\"allow_fallback\":true}',1,1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO aicli_scope_binding VALUES('app','claude','p1')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,
             output_tokens,total_cost_usd,latency_ms,status_code,created_at)
             VALUES('req-1','aicli-main','claude','m',1,2,'0.1',10,200,100)",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE proxy_request_logs SET aicli_line_id='main',aicli_profile_id='p1',
             upstream_request_id='up-1',server_group='g',billed_quota=500,billed_at=123
             WHERE request_id='req-1'",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO aicli_usage_daily VALUES('2026-11-05',10,3,0,100)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO aicli_line_health VALUES('main',1000,'ok',12,200,NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO aicli_cache(kind,fingerprint,fetched_at,body_json)
             VALUES('groups','x',1,'{\"data\":{}}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO aicli_cache(kind,fingerprint,fetched_at,body_json)
             VALUES('pricing','x',1,'{\"data\":[]}')",
            [],
        )
        .unwrap();
    }
    let outputs = vec![
        serde_json::to_string(&fixture.services.get_account().unwrap()).unwrap(),
        serde_json::to_string(&fixture.services.get_catalog().unwrap()).unwrap(),
        serde_json::to_string(&fixture.services.get_routes().await.unwrap()).unwrap(),
        serde_json::to_string(&fixture.services.list_lines().unwrap()).unwrap(),
        serde_json::to_string(&fixture.services.get_profiles().unwrap()).unwrap(),
        serde_json::to_string(&fixture.services.get_usage_summary().unwrap()).unwrap(),
        serde_json::to_string(
            &fixture
                .services
                .get_request_bill("req-1".to_owned())
                .unwrap()
                .unwrap(),
        )
        .unwrap(),
        serde_json::to_string(&fixture.services.get_privacy().unwrap()).unwrap(),
        serde_json::to_string(&fixture.services.probe_lines().await.unwrap()).unwrap(),
        serde_json::to_string(&fixture.services.refresh("all".to_owned()).unwrap()).unwrap(),
    ];
    for output in &outputs {
        assert_no_secret_material(output);
    }
    // The managed-token preview is present, masked exactly as C1-09 stores it.
    let account = serde_json::to_string(&fixture.services.get_account().unwrap()).unwrap();
    assert!(account.contains("sk-…ue-5"));
    assert!(fixture
        .services
        .get_request_bill("missing".to_owned())
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn probe_persists_results_and_backoff_skips_within_the_window() {
    let fixture = Fixture::new();
    let results = fixture.services.probe_lines().await.unwrap();
    assert_eq!(results.len(), 3);
    for result in &results {
        assert_eq!(result.status, "fail");
        assert_eq!(result.error_code.as_deref(), Some("network"));
        assert_eq!(result.next_check_at, Some(result.checked_at + 30));
    }

    // An immediate re-probe is inside the 30s window: skipped, not re-fetched.
    let skipped = fixture.services.probe_lines().await.unwrap();
    for result in &skipped {
        assert_eq!(result.status, "skipped");
        assert!(result.next_check_at.is_some());
        assert!(result.latency_ms.is_none());
    }

    let now = Utc::now().timestamp();
    // Two failures ending 60s ago: the 60s window has expired -> probe again.
    {
        let conn = fixture.db.conn.lock().unwrap();
        conn.execute("DELETE FROM aicli_line_health", []).unwrap();
        conn.execute(
            "INSERT INTO aicli_line_health VALUES('main',?1,'fail',9,NULL,'network')",
            [now - 61],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO aicli_line_health VALUES('main',?1,'fail',9,NULL,'network')",
            [now - 60],
        )
        .unwrap();
    }
    let again = fixture.services.probe_lines().await.unwrap();
    let main = again
        .iter()
        .find(|result| result.line_id == "main")
        .unwrap();
    assert_eq!(main.status, "fail");
    assert_eq!(main.next_check_at, Some(main.checked_at + 120));

    // A single failure 10s ago is inside the 30s window.
    {
        let conn = fixture.db.conn.lock().unwrap();
        conn.execute("DELETE FROM aicli_line_health", []).unwrap();
        conn.execute(
            "INSERT INTO aicli_line_health VALUES('global',?1,'fail',9,NULL,'network')",
            [now - 10],
        )
        .unwrap();
    }
    let held = fixture.services.probe_lines().await.unwrap();
    let global = held
        .iter()
        .find(|result| result.line_id == "global")
        .unwrap();
    assert_eq!(global.status, "skipped");
    assert_eq!(global.next_check_at, Some(now - 10 + 30));

    // Seven consecutive failures back off at most 600s.
    {
        let conn = fixture.db.conn.lock().unwrap();
        conn.execute("DELETE FROM aicli_line_health", []).unwrap();
        for offset in 0..7 {
            conn.execute(
                "INSERT INTO aicli_line_health VALUES('apac',?1,'fail',9,NULL,'network')",
                [now - 100 + offset],
            )
            .unwrap();
        }
    }
    let capped = fixture.services.probe_lines().await.unwrap();
    let apac = capped
        .iter()
        .find(|result| result.line_id == "apac")
        .unwrap();
    assert_eq!(apac.status, "skipped");
    assert_eq!(apac.next_check_at, Some(now - 94 + 600));

    // A recent success clears the window.
    {
        let conn = fixture.db.conn.lock().unwrap();
        conn.execute("DELETE FROM aicli_line_health", []).unwrap();
        conn.execute(
            "INSERT INTO aicli_line_health VALUES('main',?1,'ok',9,200,NULL)",
            [now - 1],
        )
        .unwrap();
    }
    let recovered = fixture.services.probe_lines().await.unwrap();
    let main = recovered
        .iter()
        .find(|result| result.line_id == "main")
        .unwrap();
    assert_eq!(main.status, "fail");
    let listed = fixture.services.list_lines().unwrap();
    let listed_main = listed
        .lines
        .iter()
        .find(|entry| entry.id == "main")
        .unwrap();
    assert_eq!(listed_main.health.as_ref().unwrap().status, "fail");
    assert!(listed_main.health.as_ref().unwrap().next_check_at.is_some());
}

#[tokio::test]
async fn refresh_validates_kinds_and_requires_a_binding() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture
            .services
            .refresh("nope".to_owned())
            .unwrap_err()
            .code,
        "invalid_input"
    );
    assert_eq!(
        fixture.services.refresh("all".to_owned()).unwrap_err().code,
        "not_bound"
    );
    fixture.seed_bound(7, ACCESS);
    // C1-06 cache-first: no row yet, so the read is empty and refresh is scheduled.
    let values = fixture.services.refresh("groups".to_owned()).unwrap();
    assert!(values.is_empty());
    fixture
        .services
        .inner
        .lock()
        .unwrap()
        .cache
        .clone()
        .unwrap()
        .wait_for_idle()
        .await;
    let cached = fixture.services.refresh("all".to_owned()).unwrap();
    assert_eq!(cached.len(), 3);
    assert_eq!(cached[0].kind, "account");
}

#[tokio::test]
async fn select_line_updates_the_binding_and_rebuilds() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture
            .services
            .select_line("nope".to_owned())
            .unwrap_err()
            .code,
        "unknown_line"
    );
    assert_eq!(
        fixture
            .services
            .select_line("apac".to_owned())
            .unwrap_err()
            .code,
        "not_bound"
    );
    fixture.seed_bound(7, ACCESS);
    fixture.services.rebuild().unwrap();
    fixture.services.select_line("apac".to_owned()).unwrap();
    assert_eq!(fixture.built.lock().unwrap().last().unwrap().1, "apac");
    assert_eq!(
        fixture.services.list_lines().unwrap().selected_line_id,
        "apac"
    );
}

#[test]
fn set_active_profile_accepts_only_app_and_global_scopes() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture
            .services
            .set_active_profile("session".to_owned(), "s".to_owned(), None)
            .unwrap_err()
            .code,
        "invalid_input"
    );
    assert_eq!(
        fixture
            .services
            .set_active_profile("account".to_owned(), String::new(), None)
            .unwrap_err()
            .code,
        "invalid_input"
    );
    assert_eq!(
        fixture
            .services
            .set_active_profile("app".to_owned(), "not-an-app".to_owned(), None)
            .unwrap_err()
            .code,
        "invalid_input"
    );
    assert_eq!(
        fixture
            .services
            .set_active_profile("global".to_owned(), "x".to_owned(), None)
            .unwrap_err()
            .code,
        "invalid_input"
    );
    assert_eq!(
        fixture
            .services
            .set_active_profile(
                "app".to_owned(),
                "claude".to_owned(),
                Some("nope".to_owned())
            )
            .unwrap_err()
            .code,
        "invalid_input"
    );
    fixture
        .services
        .set_active_profile(
            "app".to_owned(),
            "claude".to_owned(),
            Some("recommended".to_owned()),
        )
        .unwrap();
    fixture
        .services
        .set_active_profile(
            "global".to_owned(),
            String::new(),
            Some("low_cost".to_owned()),
        )
        .unwrap();
    let view = fixture.services.get_profiles().unwrap();
    assert!(view.bindings.iter().any(|binding| binding.scope == "app"
        && binding.scope_key == "claude"
        && binding.profile_id == "recommended"));
    assert!(view
        .bindings
        .iter()
        .any(|binding| binding.scope == "global" && binding.profile_id == "low_cost"));
    fixture
        .services
        .set_active_profile("app".to_owned(), "claude".to_owned(), None)
        .unwrap();
    assert!(!fixture
        .services
        .get_profiles()
        .unwrap()
        .bindings
        .iter()
        .any(|binding| binding.scope == "app"));
}

#[test]
fn privacy_requires_binding_and_validates_the_failure_mode() {
    let fixture = Fixture::new();
    let view = fixture.services.get_privacy().unwrap();
    assert!(!view.bound);
    assert_eq!(view.privacy_failure_mode, "block");
    assert_eq!(
        fixture
            .services
            .set_privacy(true, "block".to_owned(), false)
            .unwrap_err()
            .code,
        "not_bound"
    );
    fixture.seed_bound(7, ACCESS);
    assert_eq!(
        fixture
            .services
            .set_privacy(true, "degrade".to_owned(), false)
            .unwrap_err()
            .code,
        "invalid_input"
    );
    fixture
        .services
        .set_privacy(true, "pass".to_owned(), true)
        .unwrap();
    let view = fixture.services.get_privacy().unwrap();
    assert!(view.bound && view.privacy_enabled && view.allow_unscanned_media);
    assert_eq!(view.privacy_failure_mode, "pass");
}

#[test]
fn takeover_errors_carry_distinct_codes() {
    assert_eq!(
        AicliCommandError::from(TakeoverError::CcSwitchConflict).code,
        "cc_switch_conflict"
    );
    assert_eq!(
        AicliCommandError::from(TakeoverError::NoModels).code,
        "no_models"
    );
    assert_eq!(
        AicliCommandError::from(TakeoverError::TooManyModels).code,
        "too_many_models"
    );
    assert_eq!(
        AicliCommandError::from(NewApiError::UnknownLine).code,
        "unknown_line"
    );
    assert_eq!(
        AicliCommandError::from(AccountError::WrongCredentialType).code,
        "wrong_credential_type"
    );
}
