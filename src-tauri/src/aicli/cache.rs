//! Cache-first account data loading backed by the AICLI-owned SQLite tables.
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{NaiveDate, Utc};
use futures::future::BoxFuture;
use rusqlite::OptionalExtension;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::Emitter;
use tokio::sync::{Notify, Semaphore};

use super::newapi::{NewApiClient, NewApiError};
use crate::database::Database;

pub const EVENT_DATA_UPDATED: &str = "aicli-data-updated";
pub const EVENT_TAKEOVER_RESULTS: &str = "aicli-takeover-results";
pub(crate) const ACCOUNT_KINDS: [&str; 3] = ["account", "groups", "pricing"];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CachedValue {
    pub kind: String,
    pub value: Value,
    pub fetched_at: i64,
    pub stale: bool,
    pub last_error: Option<String>,
}

pub trait CacheRemote: Send + Sync {
    fn fetch<'a>(&'a self, kind: &'a str) -> BoxFuture<'a, Result<Value, NewApiError>>;
    fn fetch_usage<'a>(&'a self, day: &'a str) -> BoxFuture<'a, Result<Value, NewApiError>>;
}

pub trait CacheEvents: Send + Sync {
    fn data_updated(&self, kind: &str);
    /// C4-02：绑定后自动接管的逐项结果（不含任何令牌材质）。
    fn takeover_results(&self, payload: &str);
}

pub struct AppCacheEvents(pub tauri::AppHandle);

impl CacheEvents for AppCacheEvents {
    fn data_updated(&self, kind: &str) {
        if let Err(error) = self.0.emit(EVENT_DATA_UPDATED, kind) {
            log::warn!("Could not emit AICLI cache update event: {error}");
        }
    }

    fn takeover_results(&self, payload: &str) {
        if let Err(error) = self.0.emit(EVENT_TAKEOVER_RESULTS, payload) {
            log::warn!("Could not emit AICLI takeover results event: {error}");
        }
    }
}

pub type ReleaseAll =
    dyn Fn() -> BoxFuture<'static, Result<(), super::account::AccountError>> + Send + Sync;

pub struct CacheAccountLifecycle {
    cache: Arc<AicliCache>,
    release_all: Arc<ReleaseAll>,
}

impl CacheAccountLifecycle {
    pub fn new(cache: Arc<AicliCache>, release_all: Arc<ReleaseAll>) -> Self {
        cache.start_periodic_refresh();
        Self { cache, release_all }
    }
}

impl super::account::AccountLifecycle for CacheAccountLifecycle {
    fn refresh(&self) {
        self.cache.schedule_account_refresh();
    }

    fn release_all(&self) -> BoxFuture<'_, Result<(), super::account::AccountError>> {
        (self.release_all)()
    }
}

struct HttpRemote {
    client: NewApiClient,
}

impl CacheRemote for HttpRemote {
    fn fetch<'a>(&'a self, kind: &'a str) -> BoxFuture<'a, Result<Value, NewApiError>> {
        Box::pin(async move {
            match kind {
                "account" => self.client.user_self().await,
                "groups" => self.client.user_groups().await,
                "pricing" => self.client.pricing().await,
                _ => Err(NewApiError::InvalidInput),
            }
        })
    }

    fn fetch_usage<'a>(&'a self, day: &'a str) -> BoxFuture<'a, Result<Value, NewApiError>> {
        Box::pin(async move {
            let date = NaiveDate::parse_from_str(day, "%Y-%m-%d")
                .map_err(|_| NewApiError::InvalidInput)?;
            let start = date
                .and_hms_opt(0, 0, 0)
                .ok_or(NewApiError::InvalidInput)?
                .and_utc()
                .timestamp();
            self.client.log_daily_usage(start, start + 86_399).await
        })
    }
}

pub struct AicliCache {
    db: Arc<Database>,
    remote: Arc<dyn CacheRemote>,
    events: Arc<dyn CacheEvents>,
    inflight: Mutex<HashSet<String>>,
    errors: Mutex<HashMap<String, String>>,
    idle: Notify,
    usage_slots: Arc<Semaphore>,
    periodic_started: AtomicBool,
    /// Owner decision 2026-11-04: a refresh task created under one binding must
    /// never write rows belonging to another account after rebinding. `None`
    /// keeps the unguarded fixture behaviour; production wiring always guards.
    guard: Option<BindingGuard>,
}

struct BindingGuard {
    /// 构造时的绑定代数：bind、unbind、换线路时 +1（内存计数），令牌更换只可能伴随
    /// bind，因此代数 + 账户行足以判定绑定是否已换，写入路径不再逐次读钥匙串。
    generation: u64,
    user_id: i64,
    line: String,
}

/// 绑定代数：进程内存计数，bind / unbind / 换线路时递增。C3-11 修正。
static BINDING_GENERATION: AtomicU64 = AtomicU64::new(0);

/// 账户服务与线路切换在改变绑定的事务提交成功后调用。
pub(crate) fn bump_binding_generation() {
    BINDING_GENERATION.fetch_add(1, Ordering::SeqCst);
}

pub(crate) fn token_fingerprint(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

impl AicliCache {
    pub fn new(
        db: Arc<Database>,
        access_token: &str,
        selected_line: &str,
        events: Arc<dyn CacheEvents>,
    ) -> Result<Self, NewApiError> {
        let client = NewApiClient::new(access_token, selected_line)?;
        Ok(Self::with_remote(
            db,
            Arc::new(HttpRemote { client }),
            events,
        ))
    }

    #[cfg(feature = "test-hooks")]
    pub fn for_local_test(
        db: Arc<Database>,
        access_token: &str,
        origin: &str,
        events: Arc<dyn CacheEvents>,
    ) -> Result<Self, NewApiError> {
        let client = NewApiClient::for_local_test(access_token, origin)?;
        Ok(Self::with_remote(
            db,
            Arc::new(HttpRemote { client }),
            events,
        ))
    }

    pub fn with_remote(
        db: Arc<Database>,
        remote: Arc<dyn CacheRemote>,
        events: Arc<dyn CacheEvents>,
    ) -> Self {
        Self {
            db,
            remote,
            events,
            inflight: Mutex::new(HashSet::new()),
            errors: Mutex::new(HashMap::new()),
            idle: Notify::new(),
            usage_slots: Arc::new(Semaphore::new(2)),
            periodic_started: AtomicBool::new(false),
            guard: None,
        }
    }

    /// Production constructor: captures the binding generation and account row
    /// at construction and re-verifies both before every write, inside the same
    /// database lock as the write itself (C3-11 fix).
    pub fn guarded(
        db: Arc<Database>,
        user_id: i64,
        access_token: &str,
        selected_line: &str,
        events: Arc<dyn CacheEvents>,
    ) -> Result<Self, NewApiError> {
        let mut cache = Self::new(db, access_token, selected_line, events)?;
        cache.install_guard(user_id, selected_line);
        Ok(cache)
    }

    pub(crate) fn install_guard(&mut self, user_id: i64, selected_line: &str) {
        self.guard = Some(BindingGuard {
            generation: BINDING_GENERATION.load(Ordering::SeqCst),
            user_id,
            line: selected_line.to_owned(),
        });
    }

    /// 在持有数据库锁的写入路径内调用：先比代数（bind/unbind/换线路都会 +1），
    /// 再在同一次锁内核对账户行。不确定（锁中毒、行缺失、代数或行不一致）一律
    /// 视为已换绑，保守丢弃写入。
    fn binding_stale_locked(&self, conn: &rusqlite::Connection) -> bool {
        let Some(guard) = &self.guard else {
            return false;
        };
        if BINDING_GENERATION.load(Ordering::SeqCst) != guard.generation {
            return true;
        }
        let current = conn
            .query_row(
                "SELECT user_id,selected_line_id FROM aicli_account WHERE id=1",
                [],
                |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, String>(1)?)),
            )
            .optional();
        match current {
            Ok(Some((Some(user_id), line))) => {
                (user_id, line.as_str()) != (guard.user_id, guard.line.as_str())
            }
            _ => true,
        }
    }

    pub fn read(&self, kind: &str) -> Result<Option<CachedValue>, NewApiError> {
        let cached = {
            let conn = self.db.conn.lock().map_err(|_| NewApiError::Network)?;
            conn.query_row(
                "SELECT fetched_at,body_json FROM aicli_cache WHERE kind=?1",
                [kind],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|_| NewApiError::Network)?
        };
        let Some((fetched_at, body)) = cached else {
            return Ok(None);
        };
        let value = serde_json::from_str(&body).map_err(|_| NewApiError::InvalidResponse)?;
        let now = Utc::now().timestamp();
        let last_error = self
            .errors
            .lock()
            .ok()
            .and_then(|errors| errors.get(kind).cloned());
        Ok(Some(CachedValue {
            kind: kind.to_owned(),
            value,
            fetched_at,
            stale: now.saturating_sub(fetched_at) >= 600,
            last_error,
        }))
    }

    /// Return the persisted snapshot immediately and refresh it without blocking the caller.
    pub fn get(self: &Arc<Self>, kind: &str) -> Result<Option<CachedValue>, NewApiError> {
        if !ACCOUNT_KINDS.contains(&kind) {
            return Err(NewApiError::InvalidInput);
        }
        let cached = self.read(kind)?;
        self.schedule_refresh(kind.to_owned());
        Ok(cached)
    }

    pub fn schedule_account_refresh(self: &Arc<Self>) {
        for kind in ACCOUNT_KINDS {
            self.schedule_refresh(kind.to_owned());
        }
        let today = Utc::now().date_naive();
        for offset in 0..30 {
            let Some(day) = today.checked_sub_days(chrono::Days::new(offset)) else {
                continue;
            };
            let day = day.format("%Y-%m-%d").to_string();
            if self.usage_is_final(&day).unwrap_or(false) {
                continue;
            }
            self.schedule_usage(day);
        }
    }

    pub fn start_periodic_refresh(self: &Arc<Self>) {
        if self.periodic_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let cache = Arc::downgrade(self);
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(600));
            interval.tick().await;
            loop {
                interval.tick().await;
                let Some(cache) = cache.upgrade() else { return };
                for kind in ACCOUNT_KINDS {
                    cache.schedule_refresh(kind.to_owned());
                }
            }
        });
    }

    pub async fn wait_for_idle(&self) {
        loop {
            let notified = self.idle.notified();
            if self
                .inflight
                .lock()
                .map(|active| active.is_empty())
                .unwrap_or(true)
            {
                return;
            }
            notified.await;
        }
    }

    pub fn get_usage(self: &Arc<Self>, day: &str) -> Result<Option<CachedValue>, NewApiError> {
        let date =
            NaiveDate::parse_from_str(day, "%Y-%m-%d").map_err(|_| NewApiError::InvalidInput)?;
        let today = Utc::now().date_naive();
        let cached = self.read_usage(day)?;
        if date < today && self.usage_is_final(day)? {
            return Ok(cached);
        }
        if date <= today {
            self.schedule_usage(day.to_owned());
        }
        Ok(cached)
    }

    fn schedule_refresh(self: &Arc<Self>, kind: String) {
        if !self.begin(&kind) {
            return;
        }
        let cache = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            let result = cache.remote.fetch(&kind).await;
            cache.finish_account_refresh(&kind, result);
        });
    }

    fn schedule_usage(self: &Arc<Self>, day: String) {
        let kind = format!("usage:{day}");
        if !self.begin(&kind) {
            return;
        }
        let cache = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            let _permit = cache.usage_slots.acquire().await;
            let result = cache.remote.fetch_usage(&day).await;
            cache.finish_usage_refresh(&day, &kind, result);
        });
    }

    fn begin(&self, kind: &str) -> bool {
        self.inflight
            .lock()
            .map(|mut active| active.insert(kind.to_owned()))
            .unwrap_or(false)
    }

    fn end(&self, kind: &str) {
        if let Ok(mut active) = self.inflight.lock() {
            active.remove(kind);
        }
        self.idle.notify_one();
    }

    fn finish_account_refresh(&self, kind: &str, result: Result<Value, NewApiError>) {
        match result {
            Ok(value) => match self.write_cache(kind, &value) {
                Ok(changed) => {
                    if changed {
                        self.events.data_updated(kind);
                    }
                    if let Ok(mut errors) = self.errors.lock() {
                        errors.remove(kind);
                    }
                }
                Err(error) => self.record_error(kind, error),
            },
            Err(error) => self.record_error(kind, error),
        }
        self.end(kind);
    }

    fn finish_usage_refresh(&self, day: &str, kind: &str, result: Result<Value, NewApiError>) {
        match result {
            Ok(value) => {
                let quota = value.pointer("/data/quota").and_then(Value::as_i64);
                let requests = value.pointer("/data/requests").and_then(Value::as_i64);
                if let (Some(quota), Some(requests)) = (quota, requests) {
                    if self.write_usage(day, quota, requests).is_ok() {
                        if let Ok(mut errors) = self.errors.lock() {
                            errors.remove(kind);
                        }
                        self.events.data_updated(kind);
                    } else {
                        self.record_error(kind, NewApiError::Network);
                    }
                } else {
                    self.record_error(kind, NewApiError::InvalidResponse);
                }
            }
            Err(error) => self.record_error(kind, error),
        }
        self.end(kind);
    }

    fn record_error(&self, kind: &str, error: NewApiError) {
        if let Ok(mut errors) = self.errors.lock() {
            errors.insert(kind.to_owned(), error.to_string());
        }
    }

    fn write_cache(&self, kind: &str, value: &Value) -> Result<bool, NewApiError> {
        let mut canonical = value.clone();
        canonical.sort_all_objects();
        let body = serde_json::to_string(&canonical).map_err(|_| NewApiError::InvalidResponse)?;
        let fingerprint = format!("{:x}", Sha256::digest(body.as_bytes()));
        let now = Utc::now().timestamp();
        // C3-11 修正：守卫复核与写入在同一次数据库锁内完成，不存在两段锁之间的竞态窗口。
        let conn = self.db.conn.lock().map_err(|_| NewApiError::Network)?;
        if self.binding_stale_locked(&conn) {
            log::debug!("AICLI cache dropped a stale {kind} refresh after rebinding");
            return Ok(false);
        }
        let old = conn
            .query_row(
                "SELECT fingerprint FROM aicli_cache WHERE kind=?1",
                [kind],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|_| NewApiError::Network)?
            .flatten();
        conn.execute(
            "INSERT INTO aicli_cache(kind,fingerprint,fetched_at,body_json) VALUES(?1,?2,?3,?4)
             ON CONFLICT(kind) DO UPDATE SET fingerprint=excluded.fingerprint,
             fetched_at=excluded.fetched_at,body_json=excluded.body_json",
            rusqlite::params![kind, fingerprint, now, body],
        )
        .map_err(|_| NewApiError::Network)?;
        Ok(old.as_deref() != Some(fingerprint.as_str()))
    }

    pub fn read_usage(&self, day: &str) -> Result<Option<CachedValue>, NewApiError> {
        let kind = format!("usage:{day}");
        let last_error = self
            .errors
            .lock()
            .ok()
            .and_then(|errors| errors.get(&kind).cloned());
        let conn = self.db.conn.lock().map_err(|_| NewApiError::Network)?;
        conn.query_row(
            "SELECT quota,requests,fetched_at,is_final FROM aicli_usage_daily WHERE day=?1",
            [day],
            |row| {
                Ok(CachedValue {
                    kind,
                    value: serde_json::json!({"quota": row.get::<_, i64>(0)?, "requests": row.get::<_, i64>(1)?}),
                    fetched_at: row.get(2)?,
                    stale: !row.get::<_, bool>(3)? && Utc::now().timestamp().saturating_sub(row.get(2)?) >= 600,
                    last_error,
                })
            },
        )
        .optional()
        .map_err(|_| NewApiError::Network)
    }

    fn usage_is_final(&self, day: &str) -> Result<bool, NewApiError> {
        let conn = self.db.conn.lock().map_err(|_| NewApiError::Network)?;
        conn.query_row(
            "SELECT is_final FROM aicli_usage_daily WHERE day=?1",
            [day],
            |row| row.get::<_, bool>(0),
        )
        .optional()
        .map(|final_row| final_row.unwrap_or(false))
        .map_err(|_| NewApiError::Network)
    }

    /// 返回 `true` 表示已写入，`false` 表示绑定已换、保守丢弃。
    fn write_usage(&self, day: &str, quota: i64, requests: i64) -> Result<bool, NewApiError> {
        let final_day = day < Utc::now().format("%Y-%m-%d").to_string().as_str();
        // 同 write_cache：复核与写入共用一次数据库锁。
        let conn = self.db.conn.lock().map_err(|_| NewApiError::Network)?;
        if self.binding_stale_locked(&conn) {
            return Ok(false);
        }
        conn.execute(
            "INSERT INTO aicli_usage_daily(day,quota,requests,is_final,fetched_at) VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(day) DO UPDATE SET quota=excluded.quota,requests=excluded.requests,
             is_final=excluded.is_final,fetched_at=excluded.fetched_at",
            rusqlite::params![day, quota, requests, final_day, Utc::now().timestamp()],
        )
        .map_err(|_| NewApiError::Network)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests;
