use super::*;
use crate::aicli::secrets::{MemoryStore, SecretStore};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Barrier;

const FAST: [Duration; 3] = [
    Duration::from_millis(1),
    Duration::from_millis(1),
    Duration::from_millis(1),
];
/// 大延迟：让 worker 永远不处理，专测队列语义。
const FROZEN: [Duration; 3] = [
    Duration::from_secs(3600),
    Duration::from_secs(3600),
    Duration::from_secs(3600),
];

fn db_with_row(request_id: &str) -> Arc<Database> {
    let db = Arc::new(Database::memory().unwrap());
    db.conn
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO proxy_request_logs(request_id,provider_id,app_type,model,input_tokens,
             output_tokens,total_cost_usd,latency_ms,status_code,created_at)
             VALUES(?1,'aicli-main','claude','m',1,2,'0.1',10,200,100)",
            [request_id],
        )
        .unwrap();
    db
}

fn fixture(db: Arc<Database>) -> BillBackfill {
    let bills = BillBackfill::new(db);
    bills.set_delays_for_test(FAST);
    bills.set_query_for_test(Arc::new(|_, _, _| {
        Box::pin(async { Ok(None) }) as BoxFuture<'static, Result<Option<Value>, NewApiError>>
    }));
    bills.set_context_for_test(Arc::new(|| Some(("access".to_owned(), "main".to_owned()))));
    bills
}

fn bill_columns(
    db: &Arc<Database>,
    request_id: &str,
) -> (Option<String>, Option<i64>, Option<i64>) {
    db.conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT server_group,billed_quota,billed_at FROM proxy_request_logs WHERE request_id=?1",
            [request_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
}

fn upstream_column(db: &Arc<Database>, request_id: &str) -> Option<String> {
    db.conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT upstream_request_id FROM proxy_request_logs WHERE request_id=?1",
            [request_id],
            |row| row.get(0),
        )
        .unwrap()
}

async fn wait_for<F: Fn() -> bool>(condition: F) {
    for _ in 0..2000 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("condition never became true within the wait window");
}

/// 验收一（mock new-api 第 2 次查询才命中）：先返回空列表，再返回记账条目；
/// 上游请求 id 在请求完成时即写入行，回填只补服务端字段。
#[tokio::test]
async fn second_query_hit_fills_group_quota_and_billed_at() {
    let db = db_with_row("req-hit");
    let bills = fixture(db.clone());
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_for_query = calls.clone();
    bills.set_query_for_test(Arc::new(move |_, _, upstream| {
        let calls = calls_for_query.clone();
        let upstream = upstream.to_owned();
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(upstream, "up-hit");
            if calls.load(Ordering::SeqCst) >= 2 {
                Ok(Some(serde_json::json!({
                    "request_id": "up-hit", "group": "g-pro", "quota": 987, "created_at": 4567
                })))
            } else {
                Ok(None)
            }
        })
    }));
    bills.record("req-hit", "up-hit");
    assert_eq!(upstream_column(&db, "req-hit").as_deref(), Some("up-hit"));
    wait_for(|| bill_columns(&db, "req-hit").1.is_some()).await;
    let (group, quota, billed_at) = bill_columns(&db, "req-hit");
    assert_eq!(group.as_deref(), Some("g-pro"), "group mismatch");
    assert_eq!(quota, Some(987), "quota mismatch");
    assert_eq!(billed_at, Some(4567), "billed_at mismatch");
    assert!(calls.load(Ordering::SeqCst) >= 2);
}

/// 验收二（三次都未命中）：字段保持 NULL，条目被丢弃，队列清空。
#[tokio::test]
async fn three_misses_keep_the_bill_columns_null() {
    let db = db_with_row("req-miss");
    let bills = fixture(db.clone());
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_for_query = calls.clone();
    bills.set_query_for_test(Arc::new(move |_, _, _| {
        let calls = calls_for_query.clone();
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        })
    }));
    bills.record("req-miss", "up-miss");
    wait_for(|| calls.load(Ordering::SeqCst) >= 3).await;
    // 三次之后再无重试：给 worker 一点时间确认条目被丢弃。
    tokio::time::sleep(Duration::from_millis(50)).await;
    let columns = bill_columns(&db, "req-miss");
    assert_eq!(columns, (None, None, None), "misses must stay NULL");
    assert_eq!(upstream_column(&db, "req-miss").as_deref(), Some("up-miss"));
    assert_eq!(bills.queue_len(), 0, "exhausted entry must be dropped");
    let settled = calls.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        settled,
        "no fourth query may happen"
    );
}

/// 验收三（队列满）：丢弃最旧并只记一次警告；新条目仍在队内。
#[tokio::test]
async fn full_queue_drops_the_oldest_entry_and_warns_once() {
    let db = db_with_row("row");
    let bills = BillBackfill::new(db);
    bills.set_delays_for_test(FROZEN);
    bills.set_query_for_test(Arc::new(|_, _, _| {
        Box::pin(async { Ok(None) }) as BoxFuture<'static, Result<Option<Value>, NewApiError>>
    }));
    bills.set_context_for_test(Arc::new(|| Some(("access".to_owned(), "main".to_owned()))));

    for index in 0..QUEUE_LIMIT {
        bills.enqueue_for_test(&format!("req-{index:04}"), "up");
    }
    assert_eq!(bills.queue_len(), QUEUE_LIMIT);
    assert!(!bills.overflow_warned());
    assert_eq!(
        bills.queue_front().unwrap().0.as_str(),
        "req-0000",
        "fixture: oldest entry first"
    );

    // 满队再入一条：最旧的 req-0000 被丢弃，警告标志只置一次。
    bills.enqueue_for_test("req-new", "up-new");
    assert_eq!(bills.queue_len(), QUEUE_LIMIT);
    assert!(bills.overflow_warned());
    assert_eq!(
        bills.queue_front().unwrap().0.as_str(),
        "req-0001",
        "the oldest entry must be the one dropped"
    );

    // 再溢出多次：标志保持已警告（只记一次），持续丢最旧。
    for extra in 0..5 {
        bills.enqueue_for_test(&format!("req-extra-{extra}"), "up");
    }
    assert!(bills.overflow_warned());
    assert_eq!(
        bills.queue_front().unwrap().0.as_str(),
        "req-0006",
        "each overflow drops exactly one oldest entry"
    );
}

/// 已有服务端结果的行不被再次覆盖（billed_at IS NULL 守卫）。
#[tokio::test]
async fn existing_server_result_is_never_overwritten() {
    let db = db_with_row("req-guard");
    db.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE proxy_request_logs SET server_group='original',billed_quota=1,billed_at=1
             WHERE request_id='req-guard'",
            [],
        )
        .unwrap();
    let bills = fixture(db.clone());
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_for_query = calls.clone();
    bills.set_query_for_test(Arc::new(move |_, _, _| {
        let calls = calls_for_query.clone();
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(Some(serde_json::json!({
                "request_id": "x", "group": "late", "quota": 999, "created_at": 999
            })))
        })
    }));
    bills.record("req-guard", "up-guard");
    wait_for(|| calls.load(Ordering::SeqCst) >= 1).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let columns = bill_columns(&db, "req-guard");
    assert_eq!(columns, (Some("original".into()), Some(1), Some(1)));
}

/// 字段不全的「命中」不可用：保持 NULL，不写本地估算。
#[tokio::test]
async fn incomplete_hit_keeps_null_instead_of_filling_estimates() {
    let db = db_with_row("req-partial");
    let bills = fixture(db.clone());
    bills.set_query_for_test(Arc::new(|_, _, _| {
        Box::pin(async { Ok(Some(serde_json::json!({"request_id": "x", "group": "g"}))) })
    }));
    bills.record("req-partial", "up-partial");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(bill_columns(&db, "req-partial"), (None, None, None));
}

/// 绑定消失（未绑定/钥匙串不可读）：条目保守放弃，字段保持 NULL。
#[tokio::test]
async fn missing_binding_gives_up_without_touching_columns() {
    let db = db_with_row("req-unbound");
    let bills = BillBackfill::new(db.clone());
    bills.set_delays_for_test(FAST);
    bills.set_query_for_test(Arc::new(|_, _, _| {
        Box::pin(async { Ok(None) }) as BoxFuture<'static, Result<Option<Value>, NewApiError>>
    }));
    bills.set_context_for_test(Arc::new(|| None));
    bills.record("req-unbound", "up");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(bill_columns(&db, "req-unbound"), (None, None, None));
    assert_eq!(bills.queue_len(), 0);
}

/// 头部提取：大小写不敏感、空值忽略。
#[test]
fn header_extraction_is_case_insensitive_and_skips_empty() {
    let mut headers = http::HeaderMap::new();
    assert!(upstream_request_id(&headers).is_none());
    headers.insert("X-Oneapi-Request-Id", "up-1".parse().unwrap());
    assert_eq!(upstream_request_id(&headers).as_deref(), Some("up-1"));
    headers.insert("x-oneapi-request-id", "".parse().unwrap());
    assert!(upstream_request_id(&headers).is_none());
}

/// 生产接线的绑定上下文读数据库与钥匙串（MemoryStore 即隔离存储）。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn production_context_reads_the_current_binding() {
    let db = Arc::new(Database::memory().unwrap());
    let store = Arc::new(MemoryStore::default());
    let context = production_context(db.clone(), store.clone());
    assert!(context().is_none(), "no account row yet");
    db.conn
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO aicli_account(id,user_id,username,user_group,selected_line_id)
             VALUES(1,7,'u','g','global')",
            [],
        )
        .unwrap();
    assert!(context().is_none(), "no access token yet");
    store.set("access-token", "tok").unwrap();
    assert_eq!(context(), Some(("tok".to_owned(), "global".to_owned())));
    // 钥匙串读取在 worker 与请求路径之间共享：确认并发读不死锁。
    let barrier = Arc::new(Barrier::new(4));
    let mut joins = Vec::new();
    for _ in 0..4 {
        let context = context.clone();
        let barrier = barrier.clone();
        joins.push(std::thread::spawn(move || {
            barrier.wait();
            for _ in 0..50 {
                assert!(context().is_some());
            }
        }));
    }
    for join in joins {
        join.join().unwrap();
    }
    store.delete("access-token").unwrap();
    assert!(context().is_none());
}

/// C4-04：线路取「最终成功的 provider」——故障切换后传入切换后的 provider id
/// 即记录实际线路；非 AICLI provider、无路由决定、未知线路都返回 None。
#[test]
fn route_follows_the_final_provider_and_decision() {
    use super::super::resolver::RouteDecision;
    let decision = RouteDecision {
        public_model: "alias".to_owned(),
        upstream_model: "up".to_owned(),
        profile_id: "custom".to_owned(),
        token_id: 17,
        credential: String::new(),
        session_key: String::new(),
        wire_protocol: "anthropic".to_owned(),
        providers: Vec::new(),
        timeouts: std::collections::HashMap::new(),
    };
    assert_eq!(
        super::route_of("aicli-main", Some(&decision)),
        Some(("main".to_owned(), "custom".to_owned()))
    );
    // 故障切换后的实际成功线路
    assert_eq!(
        super::route_of("aicli-global", Some(&decision)),
        Some(("global".to_owned(), "custom".to_owned()))
    );
    assert_eq!(
        super::route_of("aicli-apac", Some(&decision)),
        Some(("apac".to_owned(), "custom".to_owned()))
    );
    // 非 AICLI provider / 无路由决定 / 未知线路
    assert_eq!(super::route_of("kimi", Some(&decision)), None);
    assert_eq!(super::route_of("aicli-main", None), None);
    assert_eq!(super::route_of("aicli-unknown", Some(&decision)), None);
    assert_eq!(super::super::lines::line_id_of_provider("aicli-nope"), None);
}
