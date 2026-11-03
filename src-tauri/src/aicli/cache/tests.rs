use super::*;
use crate::aicli::account::AccountLifecycle;
use rusqlite::Connection;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Notify;

#[derive(Default)]
struct Events(AtomicUsize);

impl CacheEvents for Events {
    fn data_updated(&self, _: &str) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct Remote {
    response: Result<Value, NewApiError>,
    calls: AtomicUsize,
    started: Notify,
    hold: bool,
}

impl Remote {
    fn new(response: Result<Value, NewApiError>) -> Self {
        Self {
            response,
            calls: AtomicUsize::new(0),
            started: Notify::new(),
            hold: false,
        }
    }

    fn holding(response: Result<Value, NewApiError>) -> Self {
        Self {
            hold: true,
            ..Self::new(response)
        }
    }
}

impl CacheRemote for Remote {
    fn fetch<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Value, NewApiError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.started.notify_one();
            if self.hold {
                std::future::pending().await
            } else {
                self.response.clone()
            }
        })
    }

    fn fetch_usage<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Value, NewApiError>> {
        Box::pin(async move { self.response.clone() })
    }
}

fn fixture(remote: Arc<dyn CacheRemote>) -> (Arc<AicliCache>, Arc<Database>, Arc<Events>) {
    let conn = Connection::open_in_memory().unwrap();
    crate::database::Database::create_tables_on_conn(&conn).unwrap();
    let db = Arc::new(Database {
        conn: Mutex::new(conn),
        log_count_cache: Mutex::new(None),
    });
    let events = Arc::new(Events::default());
    let cache = Arc::new(AicliCache::with_remote(db.clone(), remote, events.clone()));
    (cache, db, events)
}

#[tokio::test]
async fn cache_hit_returns_without_waiting_for_the_network() {
    let remote = Arc::new(Remote::holding(Err(NewApiError::Network)));
    let (cache, db, _) = fixture(remote.clone());
    db.conn
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO aicli_cache VALUES ('account','fingerprint',1000,'{\"id\":7}')",
            [],
        )
        .unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_millis(100), async {
        cache.get("account").unwrap()
    })
    .await
    .expect("cache hit waited for network")
    .unwrap();
    assert_eq!(result.value, serde_json::json!({"id":7}));
    assert!(result.stale);
    remote.started.notified().await;
}

#[tokio::test]
async fn refresh_failure_keeps_cached_value_and_exposes_error() {
    let remote = Arc::new(Remote::new(Err(NewApiError::Network)));
    let (cache, db, _) = fixture(remote);
    db.conn
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO aicli_cache VALUES ('groups','old',2000000000,'{\"g\":1}')",
            [],
        )
        .unwrap();
    cache.get("groups").unwrap();
    cache.wait_for_idle().await;
    let result = cache.read("groups").unwrap().unwrap();
    assert_eq!(result.value, serde_json::json!({"g":1}));
    assert_eq!(
        result.last_error.as_deref(),
        Some("Account API is unreachable")
    );
}

#[tokio::test]
async fn first_load_failure_returns_none_instead_of_zero() {
    let remote = Arc::new(Remote::new(Err(NewApiError::Network)));
    let (cache, _, _) = fixture(remote);
    assert!(cache.get("account").unwrap().is_none());
    cache.wait_for_idle().await;
    assert!(cache.read("account").unwrap().is_none());
}

#[tokio::test]
async fn same_kind_refresh_is_deduplicated() {
    let remote = Arc::new(Remote::holding(Err(NewApiError::Network)));
    let (cache, _, _) = fixture(remote.clone());
    for _ in 0..20 {
        cache.get("pricing").unwrap();
    }
    remote.started.notified().await;
    assert_eq!(remote.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn unchanged_content_updates_fetch_time_without_emitting_event() {
    let value = serde_json::json!({"data":{"id":7}});
    let remote = Arc::new(Remote::new(Ok(value.clone())));
    let (cache, db, events) = fixture(remote);
    let body = serde_json::to_string(&value).unwrap();
    let fingerprint = format!("{:x}", Sha256::digest(body.as_bytes()));
    db.conn
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO aicli_cache VALUES ('account',?1,1000,?2)",
            rusqlite::params![fingerprint, body],
        )
        .unwrap();
    cache.get("account").unwrap();
    cache.wait_for_idle().await;
    assert_eq!(events.0.load(Ordering::SeqCst), 0);
    let fetched_at: i64 = db
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT fetched_at FROM aicli_cache WHERE kind='account'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(fetched_at > 1000);
}

#[tokio::test]
async fn final_past_usage_is_not_requested_again() {
    let remote = Arc::new(Remote::new(Ok(serde_json::json!({}))));
    let (cache, db, _) = fixture(remote.clone());
    db.conn
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO aicli_usage_daily VALUES ('2020-01-01',10,2,1,1000)",
            [],
        )
        .unwrap();
    let result = cache.get_usage("2020-01-01").unwrap().unwrap();
    assert_eq!(result.value, serde_json::json!({"quota":10,"requests":2}));
    cache.wait_for_idle().await;
    assert_eq!(remote.calls.load(Ordering::SeqCst), 0);
}

struct UsageRemote(AtomicUsize);

impl CacheRemote for UsageRemote {
    fn fetch<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Value, NewApiError>> {
        Box::pin(async { Err(NewApiError::InvalidInput) })
    }

    fn fetch_usage<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Value, NewApiError>> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(serde_json::json!({"data":{"quota":24,"requests":3}}))
        })
    }
}

#[tokio::test]
async fn past_usage_is_fetched_once_then_marked_final() {
    let remote = Arc::new(UsageRemote(AtomicUsize::new(0)));
    let (cache, _, _) = fixture(remote.clone());
    let yesterday = (Utc::now().date_naive() - chrono::Days::new(1))
        .format("%Y-%m-%d")
        .to_string();
    assert!(cache.get_usage(&yesterday).unwrap().is_none());
    cache.wait_for_idle().await;
    let first = cache.read_usage(&yesterday).unwrap().unwrap();
    assert_eq!(first.value, serde_json::json!({"quota":24,"requests":3}));
    assert!(cache.usage_is_final(&yesterday).unwrap());
    assert!(cache.get_usage(&yesterday).unwrap().is_some());
    cache.wait_for_idle().await;
    assert_eq!(remote.0.load(Ordering::SeqCst), 1);
}

struct BackfillRemote {
    active: AtomicUsize,
    maximum: AtomicUsize,
    calls: AtomicUsize,
    two_started: Notify,
    gate: Arc<tokio::sync::Semaphore>,
}

impl CacheRemote for BackfillRemote {
    fn fetch<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Value, NewApiError>> {
        Box::pin(async { Ok(serde_json::json!({})) })
    }

    fn fetch_usage<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Value, NewApiError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum.fetch_max(active, Ordering::SeqCst);
            if active == 2 {
                self.two_started.notify_one();
            }
            let permit = self.gate.acquire().await.unwrap();
            permit.forget();
            self.active.fetch_sub(1, Ordering::SeqCst);
            Ok(serde_json::json!({"data":{"quota":0,"requests":0}}))
        })
    }
}

#[tokio::test]
async fn first_binding_backfills_thirty_days_with_at_most_two_requests() {
    let remote = Arc::new(BackfillRemote {
        active: AtomicUsize::new(0),
        maximum: AtomicUsize::new(0),
        calls: AtomicUsize::new(0),
        two_started: Notify::new(),
        gate: Arc::new(tokio::sync::Semaphore::new(0)),
    });
    let (cache, _, _) = fixture(remote.clone());
    let two_started = remote.two_started.notified();
    cache.schedule_account_refresh();
    tokio::time::timeout(std::time::Duration::from_secs(2), two_started)
        .await
        .expect("backfill did not start two requests");
    assert_eq!(remote.maximum.load(Ordering::SeqCst), 2);
    remote.gate.add_permits(30);
    tokio::time::timeout(std::time::Duration::from_secs(2), cache.wait_for_idle())
        .await
        .expect("backfill did not finish");
    assert_eq!(remote.calls.load(Ordering::SeqCst), 30);
    assert_eq!(remote.maximum.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn background_backfill_does_not_block_the_initial_account_read() {
    let remote = Arc::new(Remote::holding(Err(NewApiError::Network)));
    let (cache, _, _) = fixture(remote.clone());
    let cache = Arc::new(CacheAccountLifecycle::new(
        cache,
        Arc::new(|| Box::pin(async { Ok(()) })),
    ));
    let started = std::time::Instant::now();
    cache.refresh();
    assert!(started.elapsed() < std::time::Duration::from_millis(100));
    remote.started.notified().await;
}
