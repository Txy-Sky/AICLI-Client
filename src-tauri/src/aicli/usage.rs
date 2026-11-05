//! C4-03 账单回填：请求完成后按 `X-Oneapi-Request-Id` 向 new-api 查询记账结果。
//!
//! 请求完成时先把上游请求 id 写进 `proxy_request_logs.upstream_request_id`，再入队
//! 一个只存内存的后台任务：按 1s、3s、10s 调 `NewApiClient::log_by_request_id`，
//! 命中后把服务端的 `group`/`quota`/`created_at` 写成 `server_group`/`billed_quota`/
//! `billed_at`。三次都查不到（或账户关闭消费日志）保持 NULL，由 UI 显示
//! 「等待服务端记账」；**绝不**用本地估算值覆盖这些字段。
//!
//! 队列上限 1000：满时丢弃最旧条目并只记一次警告；不落盘，运行时销毁即清空。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use chrono::Utc;
use futures::future::BoxFuture;
use rusqlite::OptionalExtension;
use serde_json::Value;

use super::newapi::{NewApiClient, NewApiError};
use super::secrets::SecretStore;
use crate::database::Database;

pub(crate) const QUEUE_LIMIT: usize = 1000;
pub(crate) const DEFAULT_DELAYS: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(3),
    Duration::from_secs(10),
];
/// 队列空/无到期条目时的最大轮询间隔：入队通知之外的自愈兜底。
const MAX_POLL: Duration = Duration::from_millis(250);

/// 查询接缝：`(access, line, upstream_request_id)`。生产实现构造 `NewApiClient`
/// 调 `/api/log/self`；测试注入脚本化应答（即 mock new-api）。
pub type BillQuery = Arc<
    dyn Fn(&str, &str, &str) -> BoxFuture<'static, Result<Option<Value>, NewApiError>>
        + Send
        + Sync,
>;
/// 当前绑定 `(access, line)` 的读取接缝；生产实现读数据库与钥匙串。
pub type BillContext = Arc<dyn Fn() -> Option<(String, String)> + Send + Sync>;

struct Pending {
    request_id: String,
    upstream_request_id: String,
    /// 已完成的查询次数；达到 3 次仍未命中即放弃。
    attempts: u8,
    next_due: Instant,
}

struct Inner {
    db: Arc<Database>,
    queue: Mutex<VecDeque<Pending>>,
    notify: tokio::sync::Notify,
    overflow_warned: AtomicBool,
    started: AtomicBool,
    delays: Mutex<[Duration; 3]>,
    query: Mutex<Option<BillQuery>>,
    context: Mutex<Option<BillContext>>,
}

#[derive(Clone)]
pub(crate) struct BillBackfill {
    inner: Arc<Inner>,
}

/// 从上游响应头提取 `X-Oneapi-Request-Id`（流式响应在头部即可取到）。
/// `HeaderMap::get` 大小写不敏感，兼容 hyper 原样保留与 reqwest 小写两种形态。
pub(crate) fn upstream_request_id(headers: &http::HeaderMap) -> Option<String> {
    headers
        .get("x-oneapi-request-id")?
        .to_str()
        .ok()
        .map(str::to_owned)
        .filter(|value| !value.is_empty())
}

fn production_query() -> BillQuery {
    Arc::new(|access: &str, line: &str, upstream: &str| {
        let result = match NewApiClient::new(access, line) {
            Ok(client) => {
                let upstream = upstream.to_owned();
                Box::pin(async move { client.log_by_request_id(&upstream).await })
                    as BoxFuture<'static, Result<Option<Value>, NewApiError>>
            }
            Err(error) => Box::pin(async move { Err(error) }),
        };
        result
    })
}

fn production_context(
    db: Arc<Database>,
    store: Arc<dyn super::secrets::SecretStore>,
) -> BillContext {
    Arc::new(move || {
        let row = db.conn.lock().ok().and_then(|conn| {
            conn.query_row(
                "SELECT selected_line_id FROM aicli_account WHERE id=1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .ok()
            .flatten()
        });
        let line = row?;
        let access = store
            .get("access-token")
            .ok()
            .flatten()
            .filter(|value| !value.is_empty())?;
        Some((access, line))
    })
}

impl BillBackfill {
    pub(crate) fn new(db: Arc<Database>) -> Self {
        Self {
            inner: Arc::new(Inner {
                db,
                queue: Mutex::new(VecDeque::new()),
                notify: tokio::sync::Notify::new(),
                overflow_warned: AtomicBool::new(false),
                started: AtomicBool::new(false),
                delays: Mutex::new(DEFAULT_DELAYS),
                query: Mutex::new(None),
                context: Mutex::new(None),
            }),
        }
    }

    /// C4-03 生产接线：绑定读取与查询客户端；测试用下方 for_test 设置器注入。
    pub(crate) fn wire_production(
        &self,
        db: Arc<Database>,
        store: Arc<dyn super::secrets::SecretStore>,
    ) {
        *self.inner.query.lock().unwrap() = Some(production_query());
        *self.inner.context.lock().unwrap() = Some(production_context(db, store));
    }

    /// 请求完成：先把 `upstream_request_id` 写进本地行，再入队回填。
    /// 行更新失败只记日志——回填仍按 request_id 尝试，不阻断请求路径。
    pub(crate) fn record(&self, request_id: &str, upstream_request_id: &str) {
        if request_id.is_empty() || upstream_request_id.is_empty() {
            return;
        }
        if let Ok(conn) = self.inner.db.conn.lock() {
            if conn
                .execute(
                    "UPDATE proxy_request_logs SET upstream_request_id=?2 WHERE request_id=?1",
                    rusqlite::params![request_id, upstream_request_id],
                )
                .is_err()
            {
                log::warn!("AICLI bill backfill could not tag {request_id} with its upstream id");
            }
        }
        self.enqueue(Pending {
            request_id: request_id.to_owned(),
            upstream_request_id: upstream_request_id.to_owned(),
            attempts: 0,
            next_due: Instant::now() + self.delays()[0],
        });
    }

    fn enqueue(&self, entry: Pending) {
        {
            let mut queue = self.inner.queue.lock().unwrap();
            if queue.len() >= QUEUE_LIMIT {
                if let Some(dropped) = queue.pop_front() {
                    if self.inner.overflow_warned.swap(true) {
                        log::debug!(
                            "AICLI bill backfill queue full; dropping {}",
                            dropped.request_id
                        );
                    } else {
                        log::warn!(
                            "AICLI bill backfill queue is full ({}); the oldest entry was dropped \
                             and will stay unbilled",
                            QUEUE_LIMIT
                        );
                    }
                }
            }
            queue.push_back(entry);
        }
        self.start_worker();
        self.inner.notify.notify_one();
    }

    fn delays(&self) -> [Duration; 3] {
        *self.inner.delays.lock().unwrap()
    }

    fn start_worker(&self) {
        if self.inner.started.swap(true) {
            return;
        }
        let weak: Weak<Inner> = Arc::downgrade(&self.inner);
        tauri::async_runtime::spawn(async move {
            loop {
                let Some(inner) = weak.upgrade() else {
                    break;
                };
                let wake = process_due(&inner).await;
                let notified = inner.notify.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                tokio::select! {
                    _ = tokio::time::sleep(wake) => {}
                    _ = notified => {}
                }
            }
        });
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn set_delays_for_test(&self, delays: [Duration; 3]) {
        *self.inner.delays.lock().unwrap() = delays;
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn set_query_for_test(&self, query: BillQuery) {
        *self.inner.query.lock().unwrap() = Some(query);
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn set_context_for_test(&self, context: BillContext) {
        *self.inner.context.lock().unwrap() = Some(context);
    }

    #[cfg(test)]
    pub(crate) fn queue_len(&self) -> usize {
        self.inner.queue.lock().unwrap().len()
    }

    #[cfg(test)]
    pub(crate) fn queue_front(&self) -> Option<(String, String)> {
        self.inner
            .queue
            .lock()
            .unwrap()
            .front()
            .map(|entry| (entry.request_id.clone(), entry.upstream_request_id.clone()))
    }

    #[cfg(test)]
    pub(crate) fn overflow_warned(&self) -> bool {
        self.inner.overflow_warned.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    pub(crate) fn enqueue_for_test(&self, request_id: &str, upstream_request_id: &str) {
        self.enqueue(Pending {
            request_id: request_id.to_owned(),
            upstream_request_id: upstream_request_id.to_owned(),
            attempts: 0,
            next_due: Instant::now() + self.delays()[0],
        });
    }
}

enum Attempt {
    /// 命中：载荷为 /api/log/self 条目。
    Hit(Value),
    /// 未命中或瞬时错误：按剩余次数重试。
    Retry,
    /// 用尽三次或条目不可用：保持 NULL。
    GiveUp,
}

async fn process_due(inner: &Arc<Inner>) -> Duration {
    let now = Instant::now();
    let due: Vec<Pending> = {
        let mut queue = inner.queue.lock().unwrap();
        let mut keep = VecDeque::with_capacity(queue.len());
        let mut due = Vec::new();
        for entry in queue.drain(..) {
            if entry.next_due <= now {
                due.push(entry);
            } else {
                keep.push_back(entry);
            }
        }
        *queue = keep;
        due
    };
    let delays = *inner.delays.lock().unwrap();
    for mut entry in due {
        match attempt(inner, &entry).await {
            Attempt::Hit(item) => write_bill(inner, &entry.request_id, &item),
            Attempt::Retry if (entry.attempts as usize) < delays.len() - 1 => {
                entry.attempts += 1;
                entry.next_due = Instant::now() + delays[entry.attempts as usize];
                inner.queue.lock().unwrap().push_back(entry);
            }
            _ => {
                // 三次都查不到：保持 NULL，UI 显示「等待服务端记账」。
                log::debug!(
                    "AICLI bill for {} stays unbilled; server log not visible yet",
                    entry.request_id
                );
            }
        }
    }
    let queue = inner.queue.lock().unwrap();
    if queue.is_empty() {
        inner.overflow_warned.store(false, Ordering::SeqCst);
    }
    queue
        .front()
        .map(|entry| entry.next_due.saturating_duration_since(now))
        .map(|until| until.min(MAX_POLL))
        .unwrap_or(MAX_POLL)
        .max(Duration::from_millis(1))
}

async fn attempt(inner: &Arc<Inner>, entry: &Pending) -> Attempt {
    let query = inner.query.lock().unwrap().clone();
    let Some(query) = query else {
        return Attempt::GiveUp;
    };
    let context = inner.context.lock().unwrap().clone();
    let Some(context) = context else {
        return Attempt::GiveUp;
    };
    let Some((access, line)) = context() else {
        return Attempt::GiveUp;
    };
    match query(&access, &line, &entry.upstream_request_id).await {
        Ok(Some(item)) => {
            // 字段不全的条目不可用：保持 NULL，绝不用本地估算值补位。
            if item.get("group").and_then(Value::as_str).is_none()
                || item.get("quota").and_then(Value::as_i64).is_none()
            {
                return Attempt::GiveUp;
            }
            Attempt::Hit(item)
        }
        Ok(None) => Attempt::Retry,
        // 查询失败与未命中同待遇：按剩余次数重试，用尽即保持 NULL。
        Err(_) => Attempt::Retry,
    }
}

fn write_bill(inner: &Arc<Inner>, request_id: &str, item: &Value) {
    let group = item.get("group").and_then(Value::as_str);
    let quota = item.get("quota").and_then(Value::as_i64);
    let billed_at = item
        .get("created_at")
        .and_then(Value::as_i64)
        .unwrap_or_else(|| Utc::now().timestamp());
    let Ok(conn) = inner.db.conn.lock() else {
        return;
    };
    // billed_at IS NULL 守卫：只补一次，不覆盖既有服务端结果。
    let written = conn.execute(
        "UPDATE proxy_request_logs
         SET server_group=?2, billed_quota=?3, billed_at=?4
         WHERE request_id=?1 AND billed_at IS NULL",
        rusqlite::params![request_id, group, quota, billed_at],
    );
    match written {
        Ok(0) => log::debug!("AICLI bill row for {request_id} is gone or already billed"),
        Ok(_) => {}
        Err(error) => log::warn!("AICLI bill write for {request_id} failed: {error}"),
    }
}

#[cfg(test)]
mod tests;
