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

/// 捕获 C4-02 事件负载，便于断言只发一次且不含令牌材质。
#[derive(Default)]
struct RecordingEvents {
    takeover_payloads: Mutex<Vec<String>>,
}

impl CacheEvents for RecordingEvents {
    fn data_updated(&self, _: &str) {}

    fn takeover_results(&self, payload: &str) {
        self.takeover_payloads
            .lock()
            .unwrap()
            .push(payload.to_owned());
    }
}

/// C4-02 接缝假实现：记录调用并可选注入失败。
struct ProxyControlFake {
    calls: AtomicUsize,
    fail: bool,
}

impl ProxyControlFake {
    fn ok() -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            fail: false,
        })
    }
    fn failing() -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            fail: true,
        })
    }
    fn seam(self: &Arc<Self>) -> EnsureProxyFn {
        let inner = self.clone();
        Arc::new(move || {
            let inner = inner.clone();
            Box::pin(async move {
                inner.calls.fetch_add(1, Ordering::SeqCst);
                if inner.fail {
                    Err("synthetic proxy start failure".to_owned())
                } else {
                    Ok(true)
                }
            })
        })
    }
}

/// 运行时刷新发生在「代理已启动、接管尚未开始」的时刻；`on_call` 让测试在
/// 这里注入数据库故障。
type RefreshHook = Box<dyn Fn() + Send + Sync>;

struct RuntimeRefreshFake {
    calls: AtomicUsize,
    on_call: Mutex<Option<RefreshHook>>,
}

impl RuntimeRefreshFake {
    fn seam(self: &Arc<Self>) -> RuntimeRefreshFn {
        let inner = self.clone();
        Arc::new(move || {
            let inner = inner.clone();
            Box::pin(async move {
                inner.calls.fetch_add(1, Ordering::SeqCst);
                if let Some(hook) = inner.on_call.lock().unwrap().as_ref() {
                    hook();
                }
                Ok(())
            })
        })
    }

    fn set_on_call(&self, hook: impl Fn() + Send + Sync + 'static) {
        *self.on_call.lock().unwrap() = Some(Box::new(hook));
    }
}

#[derive(Default)]
struct NoEvents;

impl CacheEvents for NoEvents {
    fn data_updated(&self, _: &str) {}

    fn takeover_results(&self, _: &str) {}
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
/// `overrides` fixes per-kind replies (C4-02: keep takeover projections stable
/// while scheduled background refreshes race the bind flow).
struct ScriptedRemote {
    queue: Mutex<VecDeque<Result<Value, NewApiError>>>,
    hold_next: AtomicBool,
    gate: Notify,
    fetches: AtomicUsize,
    overrides: Mutex<std::collections::HashMap<String, Value>>,
}

impl Default for ScriptedRemote {
    fn default() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            hold_next: AtomicBool::new(false),
            gate: Notify::new(),
            fetches: AtomicUsize::new(0),
            overrides: Mutex::new(std::collections::HashMap::new()),
        }
    }
}

impl ScriptedRemote {
    fn set_override(&self, kind: &str, value: Value) {
        self.overrides
            .lock()
            .unwrap()
            .insert(kind.to_owned(), value);
    }
}

impl CacheRemote for ScriptedRemote {
    fn fetch<'a>(&'a self, kind: &'a str) -> BoxFuture<'a, Result<Value, NewApiError>> {
        Box::pin(async move {
            self.fetches.fetch_add(1, Ordering::SeqCst);
            if let Some(value) = self.overrides.lock().unwrap().get(kind) {
                return Ok(value.clone());
            }
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
    app_state: AppState,
    db: Arc<Database>,
    secrets: Arc<MemoryStore>,
    _remote: Arc<ScriptedRemote>,
    account_remote: Arc<AccountFake>,
    built: Arc<Mutex<Vec<(i64, String, String)>>>,
    caches: Arc<Mutex<Vec<Arc<AicliCache>>>>,
    proxy_control: Arc<ProxyControlFake>,
    runtime_refresh: Arc<RuntimeRefreshFake>,
    /// Keeps the database file alive for the fixture's lifetime: once the
    /// directory is removed SQLite cannot open the journal by path anymore
    /// and every write fails as readonly (observed on Linux CI; on Windows
    /// the open handle blocks deletion, hiding the bug).
    _temp: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self::with_post_bind(Arc::new(NoEvents), ProxyControlFake::ok())
    }

    fn with_post_bind(events: Arc<dyn CacheEvents>, proxy_control: Arc<ProxyControlFake>) -> Self {
        let probe: ProbeFn = Arc::new(|_| {
            let result: Result<(u16, Value), NewApiError> = Err(NewApiError::Network);
            Box::pin(async move { result })
        });
        Self::with_parts(events, proxy_control, probe)
    }

    /// F-03：探测返回给定的 `/api/status` 响应。
    fn with_status(body: Value) -> Self {
        let probe: ProbeFn = Arc::new(move |_| {
            let result: Result<(u16, Value), NewApiError> = Ok((200, body.clone()));
            Box::pin(async move { result })
        });
        Self::with_parts(Arc::new(NoEvents), ProxyControlFake::ok(), probe)
    }

    fn with_parts(
        events: Arc<dyn CacheEvents>,
        proxy_control: Arc<ProxyControlFake>,
        probe: ProbeFn,
    ) -> Self {
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
                cache.install_guard(binding.user_id, &binding.line);
                let cache = Arc::new(cache);
                caches.lock().unwrap().push(cache.clone());
                Ok(cache)
            })
        };
        let runtime_refresh = Arc::new(RuntimeRefreshFake {
            calls: AtomicUsize::new(0),
            on_call: Mutex::new(None),
        });
        let services = AicliServices::for_test(
            app_state.clone(),
            secrets.clone(),
            events,
            builder,
            probe,
            account_remote.clone(),
            proxy_control.seam(),
            runtime_refresh.seam(),
        );
        Self {
            services,
            app_state,
            db,
            secrets,
            _remote: remote,
            account_remote,
            built,
            caches,
            proxy_control,
            runtime_refresh,
            _temp: temp,
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
    // refresh("all") schedules account/groups/pricing in the background and
    // returns what is cached right now; wait for those refreshes before
    // reading again (asserting on the first return raced the spawned writes).
    fixture.services.refresh("all".to_owned()).unwrap();
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
            .set_privacy(true, "block".to_owned(), false, None)
            .unwrap_err()
            .code,
        "not_bound"
    );
    fixture.seed_bound(7, ACCESS);
    assert_eq!(
        fixture
            .services
            .set_privacy(true, "degrade".to_owned(), false, None)
            .unwrap_err()
            .code,
        "invalid_input"
    );
    fixture
        .services
        .set_privacy(true, "pass".to_owned(), true, None)
        .unwrap();
    let view = fixture.services.get_privacy().unwrap();
    assert!(view.bound && view.privacy_enabled && view.allow_unscanned_media);
    assert_eq!(view.privacy_failure_mode, "pass");
    // E-08 中文姓名：精确率不达标时开关不开放（zhname::AVAILABLE）
    assert_eq!(
        view.privacy_detect_cjk_names_available,
        crate::aicli::privacy::rules::zhname::AVAILABLE
    );
    if !crate::aicli::privacy::rules::zhname::AVAILABLE {
        assert_eq!(
            fixture
                .services
                .set_privacy(true, "pass".to_owned(), true, Some(true))
                .unwrap_err()
                .code,
            "invalid_input"
        );
        assert!(
            !fixture
                .services
                .get_privacy()
                .unwrap()
                .privacy_detect_cjk_names
        );
    }
    fixture
        .services
        .set_privacy(true, "pass".to_owned(), true, Some(false))
        .unwrap();
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
        AicliCommandError::from(TakeoverError::ProfileNeedsRepair).code,
        "profile_needs_repair"
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

// ---------------------------------------------------------------------------
// C4-02: automatic post-bind takeover of Tier 1 clients
// ---------------------------------------------------------------------------

/// C4-02 的 HOME 隔离与 `aicli/takeover.rs` 测试同一套：接管会写真实客户端
/// 配置文件，必须落在临时目录里。
struct BindHome {
    _dir: tempfile::TempDir,
    saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
    root: std::path::PathBuf,
}

impl BindHome {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let saved = [
            "HOME",
            "USERPROFILE",
            "LOCALAPPDATA",
            "CC_SWITCH_TEST_HOME",
            "XDG_CONFIG_HOME",
        ]
        .into_iter()
        .map(|key| {
            let old = std::env::var_os(key);
            std::env::set_var(
                key,
                if key == "XDG_CONFIG_HOME" {
                    dir.path().join(".config")
                } else {
                    dir.path().to_path_buf()
                },
            );
            (key, old)
        })
        .collect();
        crate::settings::reload_settings().unwrap();
        Self {
            root: dir.path().to_path_buf(),
            _dir: dir,
            saved,
        }
    }

    /// 让对应 Tier 1 客户端被判定为「已安装」。
    fn install(&self, app: &AppType) {
        let dir = match app {
            AppType::Claude => self.root.join(".claude"),
            AppType::Codex => self.root.join(".codex"),
            AppType::Gemini => self.root.join(".gemini"),
            AppType::GrokBuild => self.root.join(".grok"),
            AppType::ClaudeDesktop => {
                // Windows 上 Claude Desktop 的目录来自 %LOCALAPPDATA%\Claude（BindHome 已把
                // LOCALAPPDATA 重定向到 self.root），其余平台走 XDG 的 .config/Claude
                #[cfg(windows)]
                {
                    self.root.join("Claude")
                }
                #[cfg(not(windows))]
                {
                    self.root.join(".config").join("Claude")
                }
            }
            _ => unreachable!("not a Tier 1 client"),
        };
        std::fs::create_dir_all(dir).unwrap();
    }
}

impl Drop for BindHome {
    fn drop(&mut self) {
        for (key, old) in self.saved.drain(..) {
            if let Some(old) = old {
                std::env::set_var(key, old)
            } else {
                std::env::remove_var(key)
            }
        }
        crate::settings::reload_settings().unwrap();
    }
}

/// 组装能走到真实接管的绑定夹具：种子 Provider、可投影的缓存行、逐 kind
/// 覆盖的远端回复（后台刷新竞态不会破坏投影）、账号校验应答。
async fn bind_fixture(
    events: Arc<dyn CacheEvents>,
    proxy_control: Arc<ProxyControlFake>,
) -> Fixture {
    let fixture = Fixture::with_post_bind(events, proxy_control);
    // 接管会真的启动代理监听；固定端口会在并行测试进程间冲突，与
    // `aicli/takeover.rs` 夹具一样改用系统分配的临时端口。
    let mut config = fixture.db.get_proxy_config().await.unwrap();
    config.listen_port = 0;
    fixture.db.update_proxy_config(config).await.unwrap();
    // 代理运行时（KeyringStore，与测试聚合的 MemoryStore 是两个存储）的
    // load_cached 需要钥匙串里有绑定令牌，否则运行中快照为空、接管报 NoModels。
    use crate::aicli::secrets::SecretStore as _;
    crate::aicli::secrets::KeyringStore::default()
        .set("access-token", ACCESS)
        .unwrap();
    crate::aicli::seed::init(&fixture.db).unwrap();
    let pricing = json!({
        "auto_groups": ["fixture"],
        "data": [
            {
                "model_name": "alpha",
                "enable_groups": ["fixture"],
                "supported_endpoint_types": ["anthropic", "openai-response", "gemini"]
            }
        ]
    });
    let groups = json!({"data": {"fixture": {"ratio": 1}}});
    {
        let conn = fixture.db.conn.lock().unwrap();
        for (kind, value) in [("groups", groups.clone()), ("pricing", pricing.clone())] {
            conn.execute(
                "INSERT INTO aicli_cache(kind,fingerprint,fetched_at,body_json)
                 VALUES(?1,'fixture',1,?2)
                 ON CONFLICT(kind) DO UPDATE SET body_json=excluded.body_json",
                rusqlite::params![kind, value.to_string()],
            )
            .unwrap();
        }
    }
    fixture._remote.set_override("groups", groups);
    fixture._remote.set_override("pricing", pricing);
    fixture.account_remote.replies.lock().unwrap().push_back(Ok(
        json!({"success":true,"data":{"id":8,"username":"fixture-8","group":"default"}}),
    ));
    fixture
}

fn active_takeovers(fixture: &Fixture) -> Vec<(String, String)> {
    let conn = fixture.db.conn.lock().unwrap();
    let mut statement = conn
        .prepare("SELECT app_type,state FROM aicli_takeover ORDER BY app_type")
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

#[tokio::test]
#[serial_test::serial]
async fn bind_takes_over_all_installed_tier1_clients_and_emits_results() {
    let _home = BindHome::new();
    let events = Arc::new(RecordingEvents::default());
    let fixture = bind_fixture(events.clone(), ProxyControlFake::ok()).await;
    for app in [
        AppType::Claude,
        AppType::Codex,
        AppType::Gemini,
        AppType::GrokBuild,
        AppType::ClaudeDesktop,
    ] {
        _home.install(&app);
    }

    let result = fixture
        .services
        .bind(ACCESS.to_owned(), "main".to_owned())
        .await
        .unwrap();

    // Data refresh + proxy + compile/token seams all ran once.
    assert_eq!(fixture.proxy_control.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.runtime_refresh.calls.load(Ordering::SeqCst), 1);
    assert!(result.proxy_started);
    assert!(result.proxy_error.is_none());
    assert!(result.runtime_refresh_error.is_none());
    assert_eq!(result.takeovers.len(), 5);
    assert!(result.takeovers.iter().all(|outcome| outcome.success));

    // Every client file now points at the proxy with its own local token.
    let (proxy_url, _) = fixture
        .app_state
        .proxy_service
        .build_proxy_urls()
        .await
        .unwrap();
    let rows = active_takeovers(&fixture);
    assert_eq!(rows.len(), 5);
    assert!(rows.iter().all(|(_, state)| state == "active"));
    for app in [
        AppType::Claude,
        AppType::Codex,
        AppType::Gemini,
        AppType::GrokBuild,
        AppType::ClaudeDesktop,
    ] {
        let text = crate::aicli::takeover::managed_client_text(&app);
        assert!(text.contains(&proxy_url), "{}", app.as_str());
        let token = crate::aicli::local_auth::token_for(&fixture.db, app.as_str()).unwrap();
        assert!(text.contains(&token), "{}", app.as_str());
        assert!(!text.contains("sk-"), "{}", app.as_str());
    }

    // One event with the per-item results; nothing sensitive inside.
    let payloads = events.takeover_payloads.lock().unwrap().clone();
    assert_eq!(payloads.len(), 1);
    assert!(payloads[0].contains("\"takeovers\":["));
    assert!(payloads[0].contains("\"proxyStarted\":true"));
    assert_no_secret_material(&payloads[0]);
    assert_no_secret_material(&serde_json::to_string(&result).unwrap());
}

#[tokio::test]
#[serial_test::serial]
async fn one_client_failure_neither_breaks_the_others_nor_rolls_back_the_binding() {
    let _home = BindHome::new();
    let events = Arc::new(RecordingEvents::default());
    let fixture = bind_fixture(events, ProxyControlFake::ok()).await;
    for app in [
        AppType::Claude,
        AppType::Codex,
        AppType::Gemini,
        AppType::GrokBuild,
        AppType::ClaudeDesktop,
    ] {
        _home.install(&app);
    }
    // Grok 的 config.toml 被一个同名目录占用：读取原文件即失败，接管必然失败，
    // 其余客户端不受影响。
    std::fs::create_dir_all(_home.root.join(".grok").join("config.toml")).unwrap();

    let result = fixture
        .services
        .bind(ACCESS.to_owned(), "main".to_owned())
        .await
        .unwrap();

    assert_eq!(result.takeovers.len(), 5);
    let grok = result
        .takeovers
        .iter()
        .find(|outcome| outcome.app == "grokbuild")
        .unwrap();
    assert!(!grok.success);
    assert_eq!(grok.error_code.as_deref(), Some("client_write_failed"));
    assert!(grok.error_message.is_some());
    assert!(result
        .takeovers
        .iter()
        .filter(|outcome| outcome.app != "grokbuild")
        .all(|outcome| outcome.success));

    // Only the four healthy clients are under management; the binding stays.
    let rows = active_takeovers(&fixture);
    assert_eq!(rows.len(), 4);
    assert!(rows.iter().all(|(app, _)| app != "grokbuild"));
    let account = result.account.as_ref().expect("account view");
    assert_eq!(account.state, BindingState::Bound);
    assert_eq!(account.user_id, Some(8));
    assert!(result.account_error.is_none());
    assert!(result.takeover_targets_error.is_none());
    assert!(fixture.secrets.get("access-token").unwrap().is_some());
}

#[tokio::test]
#[serial_test::serial]
async fn rebind_targets_only_previously_managed_tier1_clients() {
    let _home = BindHome::new();
    let events = Arc::new(RecordingEvents::default());
    let fixture = bind_fixture(events, ProxyControlFake::ok()).await;
    for app in [
        AppType::Claude,
        AppType::Codex,
        AppType::Gemini,
        AppType::GrokBuild,
        AppType::ClaudeDesktop,
    ] {
        _home.install(&app);
    }
    // 非首次：只有 claude 处于接管下（另有一条非 Tier 1 的无效行，被过滤）；
    // 既有接管要求备份仍存在，否则 take_over 返回 backup_unavailable。
    fixture
        .secrets
        .set("takeover-backup:claude", "{\"version\":1,\"files\":[null]}")
        .unwrap();
    fixture
        .db
        .conn
        .lock()
        .unwrap()
        .execute_batch(
            "INSERT INTO aicli_takeover(app_type,state,backup_ref,taken_over_at)
             VALUES('claude','active','takeover-backup:claude',1);
             INSERT INTO aicli_takeover(app_type,state,backup_ref,taken_over_at)
             VALUES('opencode','active','takeover-backup:opencode',1);",
        )
        .unwrap();

    let result = fixture
        .services
        .bind(ACCESS.to_owned(), "main".to_owned())
        .await
        .unwrap();

    assert_eq!(result.takeovers.len(), 1);
    assert_eq!(result.takeovers[0].app, "claude");
    assert!(result.takeovers[0].success);
    let rows = active_takeovers(&fixture);
    assert!(rows
        .iter()
        .all(|(app, _)| app == "claude" || app == "opencode"));
}

#[tokio::test]
#[serial_test::serial]
async fn first_bind_skips_clients_that_are_not_installed() {
    let _home = BindHome::new();
    let events = Arc::new(RecordingEvents::default());
    let fixture = bind_fixture(events, ProxyControlFake::ok()).await;
    _home.install(&AppType::Codex);
    // 其余 Tier 1 目录不存在：不接管。

    let result = fixture
        .services
        .bind(ACCESS.to_owned(), "main".to_owned())
        .await
        .unwrap();

    assert_eq!(result.takeovers.len(), 1);
    assert_eq!(result.takeovers[0].app, "codex");
    assert!(result.takeovers[0].success);
}

#[tokio::test]
#[serial_test::serial]
async fn proxy_start_failure_is_reported_without_touching_clients() {
    let _home = BindHome::new();
    let events = Arc::new(RecordingEvents::default());
    let fixture = bind_fixture(events.clone(), ProxyControlFake::failing()).await;
    _home.install(&AppType::Claude);

    let result = fixture
        .services
        .bind(ACCESS.to_owned(), "main".to_owned())
        .await
        .unwrap();

    assert!(!result.proxy_started);
    assert_eq!(
        result.proxy_error.as_deref(),
        Some("synthetic proxy start failure")
    );
    assert!(result.takeovers.is_empty());
    assert_eq!(fixture.runtime_refresh.calls.load(Ordering::SeqCst), 0);
    // 客户端文件一个都没动，绑定本身保持成功。
    assert!(active_takeovers(&fixture).is_empty());
    assert_eq!(
        result.account.as_ref().expect("account view").state,
        BindingState::Bound
    );
    let payloads = events.takeover_payloads.lock().unwrap().clone();
    assert_eq!(payloads.len(), 1);
    assert!(payloads[0].contains("\"takeovers\":[]"));
}

/// 审查修正：读接管目标失败时一个都不接管（不按「表为空 = 首次绑定」接管全部
/// 已安装客户端），错误随结果返回，绑定不回滚，事件照发。
#[tokio::test]
#[serial_test::serial]
async fn unreadable_takeover_targets_take_over_nothing() {
    let _home = BindHome::new();
    let events = Arc::new(RecordingEvents::default());
    let fixture = bind_fixture(events.clone(), ProxyControlFake::ok()).await;
    for app in [AppType::Claude, AppType::Codex] {
        _home.install(&app);
    }
    let before: Vec<String> = [AppType::Claude, AppType::Codex]
        .iter()
        .map(crate::aicli::takeover::managed_client_text)
        .collect();
    let db = fixture.db.clone();
    fixture.runtime_refresh.set_on_call(move || {
        db.conn
            .lock()
            .unwrap()
            .execute_batch("ALTER TABLE aicli_takeover RENAME TO aicli_takeover_hidden")
            .unwrap();
    });

    let result = fixture
        .services
        .bind(ACCESS.to_owned(), "main".to_owned())
        .await
        .unwrap();

    assert!(result.takeovers.is_empty(), "nothing may be taken over");
    assert!(result.takeover_targets_error.is_some());
    assert!(result.account.is_some(), "binding itself stays successful");
    let payloads = events.takeover_payloads.lock().unwrap().clone();
    assert_eq!(payloads.len(), 1);
    assert!(payloads[0].contains("\"takeovers\":[]"));
    assert!(payloads[0].contains("\"takeoverTargetsError\":"));
    // 客户端配置一个都没写。
    let after: Vec<String> = [AppType::Claude, AppType::Codex]
        .iter()
        .map(crate::aicli::takeover::managed_client_text)
        .collect();
    assert_eq!(after, before, "client files must not be rewritten");
}

/// 审查修正：接管完成后读账户视图失败，逐项结果仍返回并下发，账户视图为
/// None 并带错误码，而不是整个命令报错丢掉结果。
#[tokio::test]
#[serial_test::serial]
async fn account_view_failure_keeps_takeover_results() {
    let _home = BindHome::new();
    let events = Arc::new(RecordingEvents::default());
    let fixture = bind_fixture(events.clone(), ProxyControlFake::ok()).await;
    _home.install(&AppType::Codex);
    let db = fixture.db.clone();
    // 在接管开始前隐藏账户视图依赖的 Token 表：接管本身不读它，读账户视图时失败。
    fixture.runtime_refresh.set_on_call(move || {
        db.conn
            .lock()
            .unwrap()
            .execute_batch("ALTER TABLE aicli_managed_token RENAME TO aicli_managed_token_hidden")
            .unwrap();
    });

    let result = fixture
        .services
        .bind(ACCESS.to_owned(), "main".to_owned())
        .await
        .expect("bind succeeded; the account view failure must not fail the command");

    assert!(result.account.is_none());
    assert_eq!(result.account_error.as_deref(), Some("database"));
    assert_eq!(result.takeovers.len(), 1, "per-client results survive");
    assert_eq!(result.takeovers[0].app, "codex");
    let payloads = events.takeover_payloads.lock().unwrap().clone();
    assert_eq!(payloads.len(), 1);
    assert!(payloads[0].contains("\"app\":\"codex\""));
}

// ---- F-03：额度显示设置与隐私引擎计数 ----

#[test]
fn quota_display_follows_the_web_defaults() {
    let full = json!({"data":{"quota_per_unit":250000,"quota_display_type":"CNY",
        "usd_exchange_rate":7.2,"custom_currency_symbol":" ₿ ","custom_currency_exchange_rate":3}});
    let view = AicliQuotaDisplayView::from_status(&full, 9).unwrap();
    assert_eq!(view.quota_per_unit, 250000.0);
    assert_eq!(view.quota_display_type, "CNY");
    assert_eq!(view.usd_exchange_rate, 7.2);
    assert_eq!(view.custom_currency_symbol, "₿");
    assert_eq!(view.custom_currency_exchange_rate, 3.0);
    assert_eq!(view.fetched_at, 9);

    // 缺失、非正数、未知类型、空符号：按网页端默认值
    let odd = json!({"data":{"quota_per_unit":0,"quota_display_type":"EUR",
        "usd_exchange_rate":-1,"custom_currency_symbol":"  "}});
    let view = AicliQuotaDisplayView::from_status(&odd, 1).unwrap();
    assert_eq!(view.quota_per_unit, 500000.0);
    assert_eq!(view.quota_display_type, "USD");
    assert_eq!(view.usd_exchange_rate, 1.0);
    assert_eq!(view.custom_currency_symbol, "¤");
    assert_eq!(view.custom_currency_exchange_rate, 1.0);

    assert!(AicliQuotaDisplayView::from_status(&json!({"success":true}), 1).is_none());
}

#[tokio::test]
async fn successful_probe_stores_the_quota_display_for_the_account_view() {
    let fixture = Fixture::with_status(json!({"success":true,"data":{
        "quota_per_unit":500000,"quota_display_type":"TOKENS","system_name":"AICLI"}}));
    fixture.seed_bound(7, ACCESS);
    assert!(fixture
        .services
        .get_account()
        .unwrap()
        .quota_display
        .is_none());

    let results = fixture.services.probe_lines().await.unwrap();
    assert!(results.iter().all(|result| result.status == "ok"));
    let display = fixture
        .services
        .get_account()
        .unwrap()
        .quota_display
        .unwrap();
    assert_eq!(display.quota_display_type, "TOKENS");
    assert_eq!(display.quota_per_unit, 500000.0);
    assert!(display.fetched_at > 0);
    // 只存显示设置，不存其余状态字段
    let body: String = fixture
        .db
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT body_json FROM aicli_cache WHERE kind='status'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!body.contains("system_name"), "{body}");
}

#[test]
fn privacy_view_has_no_engine_counts_while_the_proxy_is_stopped() {
    let fixture = Fixture::new();
    fixture.seed_bound(7, ACCESS);
    assert!(fixture.services.get_privacy().unwrap().engine.is_none());
}
