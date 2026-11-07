//! C3-11 Tauri command layer for the AICLI account lifecycle.
//!
//! Serialization contract: command returns carry public account fields and
//! masked token previews only — never the access token, never an `sk-` key.
//! `aicli_bind` parameters are never logged. Read commands follow C1-06: the
//! persisted cache is returned immediately and refreshed in the background.

use std::str::FromStr;
use std::sync::{Arc, Mutex, Weak};

use chrono::Utc;
use futures::future::BoxFuture;
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use tauri::State;

use crate::aicli::account::{
    http_remote, AccountError, AccountLifecycle, AccountRemote, AccountService, BindingState,
};
use crate::aicli::cache::{
    token_fingerprint, AicliCache, CacheAccountLifecycle, CacheEvents, CacheRemote, CachedValue,
    ReleaseAll, ACCOUNT_KINDS,
};
use crate::aicli::catalog::Catalog;
use crate::aicli::lines::{line, LINES};
use crate::aicli::newapi::{self, NewApiError};
use crate::aicli::profiles::{self, ModelRoute, Profile, ProfileError, ProfileStore, RepairStatus};
use crate::aicli::runtime::{AicliRuntime, RoutesSnapshot, RuntimeError};
use crate::aicli::secrets::SecretStore;
use crate::aicli::takeover::{self, TakeoverError};
use crate::app_config::AppType;
use crate::database::Database;
use crate::store::AppState;

// ---------------------------------------------------------------------------
// Error surface: a stable machine code per failure so the frontend can react
// (for example the cc-switch confirmation dialog) without string matching.
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct AicliCommandError {
    pub code: String,
    pub message: String,
}

impl AicliCommandError {
    fn invalid_input(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_input".into(),
            message: message.into(),
        }
    }
    fn not_bound() -> Self {
        Self {
            code: "not_bound".into(),
            message: "Bind an account first".into(),
        }
    }
    fn database() -> Self {
        Self {
            code: "database".into(),
            message: "Local AICLI storage is unavailable".into(),
        }
    }
    fn secret_store() -> Self {
        Self {
            code: "secret_store".into(),
            message: "The credential store is unavailable".into(),
        }
    }
}

impl From<AccountError> for AicliCommandError {
    fn from(error: AccountError) -> Self {
        let code = match error {
            AccountError::WrongCredentialType => "wrong_credential_type",
            AccountError::NotBound => "not_bound",
            AccountError::Database => "database",
            AccountError::CredentialRollback => "credential_rollback",
            AccountError::Restore => "restore",
            AccountError::Api(_) | AccountError::Secret(_) => "account_api",
        };
        Self {
            code: code.into(),
            message: error.to_string(),
        }
    }
}

impl From<NewApiError> for AicliCommandError {
    fn from(error: NewApiError) -> Self {
        let code = match error {
            NewApiError::TokenInvalid => "token_invalid",
            NewApiError::Http(_) => "http",
            NewApiError::RateLimited(_) => "rate_limited",
            NewApiError::Rejected => "rejected",
            NewApiError::InvalidResponse => "invalid_response",
            NewApiError::Network => "network",
            NewApiError::InvalidInput => "invalid_input",
            NewApiError::UnknownLine => "unknown_line",
        };
        Self {
            code: code.into(),
            message: error.to_string(),
        }
    }
}

impl From<TakeoverError> for AicliCommandError {
    fn from(error: TakeoverError) -> Self {
        let code = match error {
            TakeoverError::CcSwitchConflict => "cc_switch_conflict",
            TakeoverError::NoModels => "no_models",
            TakeoverError::ProfileNeedsRepair => "profile_needs_repair",
            TakeoverError::TooManyModels => "too_many_models",
            TakeoverError::Unsupported => "unsupported_client",
            TakeoverError::Backup => "backup_unavailable",
            TakeoverError::Client => "client_write_failed",
            TakeoverError::State => "takeover_state",
            TakeoverError::Verification => "restore_verification",
        };
        Self {
            code: code.into(),
            message: error.to_string(),
        }
    }
}

impl From<ProfileError> for AicliCommandError {
    fn from(error: ProfileError) -> Self {
        let code = match error {
            ProfileError::Database => "database",
            ProfileError::Invalid => "invalid_profile",
            ProfileError::ReadOnly => "read_only",
            ProfileError::RevisionConflict => "revision_conflict",
            ProfileError::MissingGroups(_) => "missing_groups",
            ProfileError::UnresolvedConflicts(_) => "unresolved_conflicts",
            ProfileError::DuplicateAlias => "duplicate_alias",
            ProfileError::TooManyGroups { .. } => "too_many_groups",
            ProfileError::NeedsRepair(_) => "needs_repair",
        };
        Self {
            code: code.into(),
            message: error.to_string(),
        }
    }
}

impl From<RuntimeError> for AicliCommandError {
    fn from(error: RuntimeError) -> Self {
        match error {
            RuntimeError::Storage => Self::database(),
            RuntimeError::Api(inner) => inner.into(),
            RuntimeError::Profile(inner) => inner.into(),
            RuntimeError::Secret(_) => Self::secret_store(),
            RuntimeError::Token(_) => Self {
                code: "token".into(),
                message: error.to_string(),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// DTOs. Every field is public routing/billing data; masked previews only.
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct AicliManagedTokenView {
    pub token_id: i64,
    pub name: String,
    pub masked: String,
    pub created_by_client: bool,
}

#[derive(Serialize)]
pub struct AicliAccountView {
    pub state: BindingState,
    pub user_id: Option<i64>,
    pub username: Option<String>,
    pub user_group: Option<String>,
    pub selected_line_id: Option<String>,
    pub active_profile_id: Option<String>,
    pub managed_tokens: Vec<AicliManagedTokenView>,
}

/// C4-02：绑定后自动接管单个 Tier 1 客户端的结果；错误只有机器码和短文案，不含令牌材质。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AicliTakeoverOutcome {
    pub app: String,
    pub success: bool,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

/// C4-02：绑定命令的返回与 `aicli-takeover-results` 事件负载。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AicliBindResult {
    /// 绑定后的账户视图；读取失败时为 None 并记 `account_error`（绑定本身已成功，
    /// 逐项接管结果照常返回与下发）。
    pub account: Option<AicliAccountView>,
    pub account_error: Option<String>,
    pub proxy_started: bool,
    pub proxy_error: Option<String>,
    pub runtime_refresh_error: Option<String>,
    /// 读取接管目标失败：一个客户端都不接管（不按首次绑定扩大范围）。
    pub takeover_targets_error: Option<String>,
    pub takeovers: Vec<AicliTakeoverOutcome>,
}

#[derive(Serialize)]
pub struct AicliLineHealthView {
    pub checked_at: i64,
    pub status: String,
    pub latency_ms: Option<i64>,
    pub http_status: Option<i64>,
    pub error_code: Option<String>,
    pub next_check_at: Option<i64>,
}

#[derive(Serialize)]
pub struct AicliLineView {
    pub id: String,
    pub label_key: String,
    pub transport: String,
    pub priority: u16,
    pub first_byte_s: u64,
    pub idle_s: u64,
    pub stream_total_s: u64,
    pub non_stream_s: u64,
    pub health: Option<AicliLineHealthView>,
}

#[derive(Serialize)]
pub struct AicliLinesView {
    pub selected_line_id: String,
    pub lines: Vec<AicliLineView>,
}

#[derive(Serialize)]
pub struct AicliProbeView {
    pub line_id: String,
    pub status: String,
    pub checked_at: i64,
    pub latency_ms: Option<i64>,
    pub http_status: Option<i64>,
    pub error_code: Option<String>,
    pub next_check_at: Option<i64>,
}

#[derive(Serialize)]
pub struct AicliScopeBindingView {
    pub scope: String,
    pub scope_key: String,
    pub profile_id: String,
}

#[derive(Serialize)]
pub struct AicliProfilesView {
    pub profiles: Vec<Profile>,
    pub presets: Vec<Profile>,
    pub bindings: Vec<AicliScopeBindingView>,
    pub active_profile_id: Option<String>,
}

#[derive(Serialize)]
pub struct AicliRouteProfileView {
    pub id: String,
    pub name: String,
    pub source: String,
    pub preset_id: Option<String>,
    pub revision: i64,
    pub routes: Vec<ModelRoute>,
    pub repair: RepairStatus,
}

#[derive(Serialize)]
pub struct AicliRoutesView {
    pub fingerprint: String,
    pub active_profile_id: Option<String>,
    pub selected_line_id: String,
    pub profiles: Vec<AicliRouteProfileView>,
}

#[derive(Serialize)]
pub struct AicliUsageDayView {
    pub day: String,
    pub quota: i64,
    pub requests: i64,
    pub is_final: bool,
    pub fetched_at: i64,
    pub stale: bool,
}

#[derive(Serialize)]
pub struct AicliUsageSummaryView {
    pub days: Vec<AicliUsageDayView>,
    pub total_quota: i64,
    pub total_requests: i64,
}

#[derive(Serialize)]
pub struct AicliRequestBillView {
    pub request_id: String,
    pub created_at: i64,
    pub app_type: String,
    pub model: String,
    pub status_code: i64,
    pub line_id: Option<String>,
    pub profile_id: Option<String>,
    pub upstream_request_id: Option<String>,
    pub server_group: Option<String>,
    pub billed_quota: Option<i64>,
    pub billed_at: Option<i64>,
}

#[derive(Serialize)]
pub struct AicliPrivacyView {
    pub bound: bool,
    pub privacy_enabled: bool,
    pub privacy_failure_mode: String,
    pub allow_unscanned_media: bool,
}

// ---------------------------------------------------------------------------
// Line probe: unauthenticated /api/status with failure backoff.
// ---------------------------------------------------------------------------

/// 30s, 60s, 120s... capped at 600s after `failures` consecutive failures.
fn backoff_seconds(failures: u32) -> i64 {
    if failures == 0 {
        return 0;
    }
    30i64
        .checked_shl(failures - 1)
        .map(|delay| delay.min(600))
        .unwrap_or(600)
}

fn probe_error_code(error: &NewApiError) -> &'static str {
    match error {
        NewApiError::TokenInvalid => "token_invalid",
        NewApiError::Http(_) => "http",
        NewApiError::RateLimited(_) => "rate_limited",
        NewApiError::Rejected => "rejected",
        NewApiError::InvalidResponse => "invalid_response",
        NewApiError::Network => "network",
        NewApiError::InvalidInput => "invalid_input",
        NewApiError::UnknownLine => "unknown_line",
    }
}

fn consecutive_failures(conn: &Connection, line_id: &str) -> Result<u32, AicliCommandError> {
    let mut statement = conn
        .prepare(
            "SELECT status FROM aicli_line_health WHERE line_id=?1
             ORDER BY checked_at DESC LIMIT 32",
        )
        .map_err(|_| AicliCommandError::database())?;
    let statuses = statement
        .query_map([line_id], |row| row.get::<_, String>(0))
        .map_err(|_| AicliCommandError::database())?;
    let mut failures = 0u32;
    for status in statuses {
        if status.map_err(|_| AicliCommandError::database())? == "fail" {
            failures += 1;
        } else {
            break;
        }
    }
    Ok(failures)
}

// ---------------------------------------------------------------------------
// The aggregate service: rebuilds the bound cache whenever the binding key
// `(user_id, line, token fingerprint)` changes, mirroring the runtime's
// BoundTokenService pattern, and owns the C1-06 lifecycle wiring.
// ---------------------------------------------------------------------------

type ProbeFn =
    Arc<dyn Fn(&str) -> BoxFuture<'static, Result<(u16, Value), NewApiError>> + Send + Sync>;

/// C4-02 接缝：确保代理运行。返回 `true` 表示本次启动，`false` 表示已在运行；
/// 测试注入假实现，不真的监听端口。
type EnsureProxyFn = Arc<dyn Fn() -> BoxFuture<'static, Result<bool, String>> + Send + Sync>;

/// C4-02 接缝：代理运行时的刷新（拉取账户/分组/价格 → 编译含默认配置集在内的全部
/// 配置集 → 批量取得 Token，见 `AicliRuntime::refresh`）。
type RuntimeRefreshFn = Arc<dyn Fn() -> BoxFuture<'static, Result<(), String>> + Send + Sync>;

pub(crate) struct CacheBinding {
    pub db: Arc<Database>,
    pub user_id: i64,
    pub access: String,
    pub line: String,
    pub events: Arc<dyn CacheEvents>,
}

type CacheBuilder = Arc<dyn Fn(CacheBinding) -> Result<Arc<AicliCache>, NewApiError> + Send + Sync>;

fn production_cache_builder() -> CacheBuilder {
    Arc::new(|binding| {
        let cache = AicliCache::guarded(
            binding.db,
            binding.user_id,
            &binding.access,
            &binding.line,
            binding.events,
        )?;
        Ok(Arc::new(cache))
    })
}

fn real_probe() -> ProbeFn {
    Arc::new(|line_id| {
        let line_id = line_id.to_owned();
        Box::pin(async move { newapi::public_status(&line_id).await })
    })
}

fn real_proxy_control(app_state: &AppState) -> EnsureProxyFn {
    let service = app_state.proxy_service.clone();
    Arc::new(move || {
        let service = service.clone();
        Box::pin(async move {
            if service.is_running().await {
                Ok(false)
            } else {
                service.start().await.map(|_| true)
            }
        })
    })
}

fn real_runtime_refresh(app_state: &AppState) -> RuntimeRefreshFn {
    let service = app_state.proxy_service.clone();
    Arc::new(move || {
        let service = service.clone();
        Box::pin(async move {
            match service.aicli_runtime().await {
                Some(runtime) => runtime.refresh().await.map_err(|error| error.to_string()),
                None => Err("AICLI proxy runtime unavailable".to_owned()),
            }
        })
    })
}

/// Restores every active takeover (C3-10) — the mandatory release callback
/// injected into the account lifecycle, independent of cache state.
async fn release_all_takeovers(
    app_state: AppState,
    secrets: Arc<dyn SecretStore>,
) -> Result<(), AccountError> {
    let apps: Vec<String> = {
        let conn = app_state
            .db
            .conn
            .lock()
            .map_err(|_| AccountError::Database)?;
        let mut statement = conn
            .prepare("SELECT app_type FROM aicli_takeover ORDER BY app_type")
            .map_err(|_| AccountError::Database)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|_| AccountError::Database)?;
        rows.collect::<Result<_, _>>()
            .map_err(|_| AccountError::Database)?
    };
    for id in apps {
        let app = AppType::from_str(&id).map_err(|_| AccountError::Database)?;
        takeover::release(&app_state, secrets.as_ref(), &app)
            .await
            .map_err(|_| AccountError::Restore)?;
    }
    Ok(())
}

/// C1-06 lifecycle, rebind-scoped: whenever the account service reports a
/// change (bind) the aggregate is rebuilt first, then the current cache
/// schedules its refresh. `release_all` always runs the real takeover restore.
struct RebindLifecycle {
    services: Weak<AicliServices>,
    release: Arc<ReleaseAll>,
}

impl AccountLifecycle for RebindLifecycle {
    fn refresh(&self) {
        let Some(services) = self.services.upgrade() else {
            return;
        };
        if services.rebuild().is_err() {
            return;
        }
        if let Some(lifecycle) = services
            .inner
            .lock()
            .ok()
            .and_then(|inner| inner.lifecycle.clone())
        {
            lifecycle.refresh();
        }
    }

    fn release_all(&self) -> BoxFuture<'_, Result<(), AccountError>> {
        (self.release)()
    }
}

struct Inner {
    key: Option<(i64, String, String)>,
    cache: Option<Arc<AicliCache>>,
    lifecycle: Option<Arc<CacheAccountLifecycle>>,
    account: Arc<AccountService>,
}

pub struct AicliServices {
    app_state: AppState,
    db: Arc<Database>,
    secrets: Arc<dyn SecretStore>,
    events: Arc<dyn CacheEvents>,
    cache_builder: CacheBuilder,
    probe: ProbeFn,
    ensure_proxy: EnsureProxyFn,
    runtime_refresh: RuntimeRefreshFn,
    inner: Mutex<Inner>,
}

/// Fails every fetch; used for reads while unbound, never scheduled.
struct OfflineRemote;

impl CacheRemote for OfflineRemote {
    fn fetch<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Value, NewApiError>> {
        Box::pin(async { Err(NewApiError::Network) })
    }
    fn fetch_usage<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Value, NewApiError>> {
        Box::pin(async { Err(NewApiError::Network) })
    }
}

impl AicliServices {
    pub fn new(
        app_state: AppState,
        secrets: Arc<dyn SecretStore>,
        events: Arc<dyn CacheEvents>,
    ) -> Arc<Self> {
        Self::build(
            app_state,
            secrets,
            events,
            production_cache_builder(),
            real_probe(),
            http_remote(),
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn for_test(
        app_state: AppState,
        secrets: Arc<dyn SecretStore>,
        events: Arc<dyn CacheEvents>,
        cache_builder: CacheBuilder,
        probe: ProbeFn,
        account_remote: Arc<dyn AccountRemote>,
        ensure_proxy: EnsureProxyFn,
        runtime_refresh: RuntimeRefreshFn,
    ) -> Arc<Self> {
        Self::build_with_post_bind(
            app_state,
            secrets,
            events,
            cache_builder,
            probe,
            account_remote,
            ensure_proxy,
            runtime_refresh,
        )
    }

    fn build(
        app_state: AppState,
        secrets: Arc<dyn SecretStore>,
        events: Arc<dyn CacheEvents>,
        cache_builder: CacheBuilder,
        probe: ProbeFn,
        account_remote: Arc<dyn AccountRemote>,
    ) -> Arc<Self> {
        let ensure_proxy = real_proxy_control(&app_state);
        let runtime_refresh = real_runtime_refresh(&app_state);
        Self::build_with_post_bind(
            app_state,
            secrets,
            events,
            cache_builder,
            probe,
            account_remote,
            ensure_proxy,
            runtime_refresh,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build_with_post_bind(
        app_state: AppState,
        secrets: Arc<dyn SecretStore>,
        events: Arc<dyn CacheEvents>,
        cache_builder: CacheBuilder,
        probe: ProbeFn,
        account_remote: Arc<dyn AccountRemote>,
        ensure_proxy: EnsureProxyFn,
        runtime_refresh: RuntimeRefreshFn,
    ) -> Arc<Self> {
        let db = app_state.db.clone();
        let release: Arc<ReleaseAll> = {
            let app_state = app_state.clone();
            let secrets = secrets.clone();
            Arc::new(move || {
                let app_state = app_state.clone();
                let secrets = secrets.clone();
                Box::pin(release_all_takeovers(app_state, secrets))
            })
        };
        Arc::new_cyclic(|weak| {
            let lifecycle = Arc::new(RebindLifecycle {
                services: weak.clone(),
                release: release.clone(),
            });
            let account = Arc::new(
                AccountService::with_remote(db.clone(), secrets.clone(), lifecycle, account_remote)
                    .expect("account service construction only reads local state"),
            );
            AicliServices {
                app_state,
                db,
                secrets,
                events,
                cache_builder,
                probe,
                ensure_proxy,
                runtime_refresh,
                inner: Mutex::new(Inner {
                    key: None,
                    cache: None,
                    lifecycle: None,
                    account,
                }),
            }
        })
    }

    /// Read `(user_id, selected_line, access token)`; missing pieces mean unbound.
    fn current_binding(
        db: &Database,
        secrets: &dyn SecretStore,
    ) -> Result<Option<(i64, String, String)>, AicliCommandError> {
        let row = db
            .conn
            .lock()
            .map_err(|_| AicliCommandError::database())?
            .query_row(
                "SELECT user_id,selected_line_id FROM aicli_account WHERE id=1",
                [],
                |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|_| AicliCommandError::database())?;
        let Some((Some(user_id), selected_line)) = row else {
            return Ok(None);
        };
        let access = secrets
            .get("access-token")
            .map_err(|_| AicliCommandError::secret_store())?
            .filter(|token| !token.is_empty());
        Ok(access.map(|access| (user_id, selected_line, access)))
    }

    /// Swap the bound cache when the binding key changed; no-op otherwise.
    fn rebuild(&self) -> Result<(), AicliCommandError> {
        let binding = Self::current_binding(&self.db, self.secrets.as_ref())?;
        let key = binding.as_ref().map(|(user_id, selected_line, access)| {
            (*user_id, selected_line.clone(), token_fingerprint(access))
        });
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| AicliCommandError::database())?;
        if inner.key == key {
            return Ok(());
        }
        match binding {
            None => {
                inner.key = None;
                inner.cache = None;
                inner.lifecycle = None;
            }
            Some((user_id, selected_line, access)) => {
                let cache = (self.cache_builder)(CacheBinding {
                    db: self.db.clone(),
                    user_id,
                    access: access.clone(),
                    line: selected_line.clone(),
                    events: self.events.clone(),
                })?;
                // CacheAccountLifecycle::new starts the 600s periodic refresh.
                let lifecycle = Arc::new(CacheAccountLifecycle::new(
                    cache.clone(),
                    release_lifecycle_callback(&self.app_state, self.secrets.clone()),
                ));
                inner.key = Some((user_id, selected_line, token_fingerprint(&access)));
                inner.cache = Some(cache);
                inner.lifecycle = Some(lifecycle);
            }
        }
        Ok(())
    }

    fn live_cache(&self) -> Result<Arc<AicliCache>, AicliCommandError> {
        self.rebuild()?;
        self.inner
            .lock()
            .map_err(|_| AicliCommandError::database())?
            .cache
            .clone()
            .ok_or_else(AicliCommandError::not_bound)
    }

    fn read_cache(&self) -> Arc<AicliCache> {
        let _ = self.rebuild();
        let bound = self.inner.lock().ok().and_then(|inner| inner.cache.clone());
        bound.unwrap_or_else(|| {
            Arc::new(AicliCache::with_remote(
                self.db.clone(),
                Arc::new(OfflineRemote),
                self.events.clone(),
            ))
        })
    }

    fn account_service(&self) -> Arc<AccountService> {
        self.inner
            .lock()
            .ok()
            .map(|inner| inner.account.clone())
            .expect("account service is constructed with the aggregate")
    }

    /// App startup: rebuild for an existing binding and schedule one refresh.
    pub fn startup(&self) {
        if self.rebuild().is_err() {
            return;
        }
        if let Some(lifecycle) = self
            .inner
            .lock()
            .ok()
            .and_then(|inner| inner.lifecycle.clone())
        {
            lifecycle.refresh();
        }
    }

    // -- Account ---------------------------------------------------------------

    pub fn get_account(&self) -> Result<AicliAccountView, AicliCommandError> {
        let account_service = self.account_service();
        let state = account_service.state();
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|_| AicliCommandError::database())?;
        let identity = conn
            .query_row(
                "SELECT user_id,username,user_group,selected_line_id,active_profile_id
                 FROM aicli_account WHERE id=1",
                [],
                |row| {
                    Ok((
                        row.get::<_, Option<i64>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| AicliCommandError::database())?;
        let tokens = conn
            .prepare("SELECT token_id,name,masked,created_by_client FROM aicli_managed_token ORDER BY token_id")
            .map_err(|_| AicliCommandError::database())?
            .query_map([], |row| {
                Ok(AicliManagedTokenView {
                    token_id: row.get(0)?,
                    name: row.get(1)?,
                    masked: row.get(2)?,
                    created_by_client: row.get(3)?,
                })
            })
            .map_err(|_| AicliCommandError::database())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| AicliCommandError::database())?;
        let (user_id, username, user_group, selected_line_id, active_profile_id) = identity
            .map(
                |(user_id, username, user_group, selected_line_id, active_profile_id)| {
                    (
                        user_id,
                        username.unwrap_or_default(),
                        user_group.unwrap_or_default(),
                        selected_line_id,
                        active_profile_id,
                    )
                },
            )
            .unwrap_or((None, String::new(), String::new(), None, None));
        Ok(AicliAccountView {
            state,
            user_id,
            username: (state != BindingState::NotBound).then_some(username),
            user_group: (state != BindingState::NotBound).then_some(user_group),
            selected_line_id,
            active_profile_id,
            managed_tokens: tokens,
        })
    }

    /// `access_token` and `line` are never logged anywhere in this path.
    ///
    /// C4-02：绑定成功后刷新数据（账户服务已触发 C1-06 生命周期刷新）→ 确保代理运行
    /// → 运行时刷新（编译含默认配置集在内的全部配置集并批量取得 Token）→ 只接管
    /// 目标 Tier 1 客户端 → 发 `aicli-takeover-results` 事件。任一客户端失败不影响
    /// 其他客户端，也不回滚绑定；结果逐项返回。
    pub async fn bind(
        &self,
        access_token: String,
        line_id: String,
    ) -> Result<AicliBindResult, AicliCommandError> {
        self.account_service().bind(&access_token, &line_id).await?;
        self.rebuild()?;

        let mut proxy_started = false;
        let mut proxy_error = None;
        match (self.ensure_proxy)().await {
            Ok(started) => proxy_started = started,
            Err(error) => {
                log::error!("AICLI post-bind proxy start failed: {error}");
                proxy_error = Some(error);
            }
        }
        let mut runtime_refresh_error = None;
        let mut takeover_targets_error = None;
        let mut takeovers = Vec::new();
        if proxy_error.is_none() {
            if let Err(error) = (self.runtime_refresh)().await {
                // 缓存投影仍可能可用；逐项接管自行成败，不阻断绑定。
                log::warn!("AICLI post-bind runtime refresh failed: {error}");
                runtime_refresh_error = Some(error);
            }
            let targets = match takeover::bind_targets(&self.db) {
                Ok(targets) => targets,
                Err(error) => {
                    log::error!("AICLI post-bind takeover targets unreadable: {error}");
                    takeover_targets_error = Some(error);
                    Vec::new()
                }
            };
            for app in targets {
                let outcome =
                    match takeover::take_over(&self.app_state, self.secrets.as_ref(), &app, false)
                        .await
                    {
                        Ok(()) => AicliTakeoverOutcome {
                            app: app.as_str().to_owned(),
                            success: true,
                            error_code: None,
                            error_message: None,
                        },
                        Err(error) => {
                            let mapped = AicliCommandError::from(error);
                            log::warn!(
                                "AICLI post-bind takeover failed for {}: {}",
                                app.as_str(),
                                mapped.code
                            );
                            AicliTakeoverOutcome {
                                app: app.as_str().to_owned(),
                                success: false,
                                error_code: Some(mapped.code),
                                error_message: Some(mapped.message),
                            }
                        }
                    };
                takeovers.push(outcome);
            }
        }

        // 代理与接管都已发生：读账户视图失败不能吞掉逐项结果，照常返回并发事件。
        let (account, account_error) = match self.get_account() {
            Ok(view) => (Some(view), None),
            Err(error) => {
                log::warn!("AICLI post-bind account view failed: {}", error.code);
                (None, Some(error.code))
            }
        };
        let result = AicliBindResult {
            account,
            account_error,
            proxy_started,
            proxy_error,
            runtime_refresh_error,
            takeover_targets_error,
            takeovers,
        };
        match serde_json::to_string(&result) {
            Ok(payload) => self.events.takeover_results(&payload),
            Err(error) => log::warn!("Could not serialize AICLI bind result event: {error}"),
        }
        Ok(result)
    }

    pub async fn unbind(&self, delete_created_tokens: bool) -> Result<(), AicliCommandError> {
        self.account_service().unbind(delete_created_tokens).await?;
        self.rebuild()?;
        Ok(())
    }

    pub fn refresh(&self, kind: String) -> Result<Vec<CachedValue>, AicliCommandError> {
        if !matches!(kind.as_str(), "account" | "groups" | "pricing" | "all") {
            return Err(AicliCommandError::invalid_input(
                "kind must be one of account, groups, pricing, all",
            ));
        }
        let cache = self.live_cache()?;
        match kind.as_str() {
            "account" | "groups" | "pricing" => Ok(cache.get(&kind)?.into_iter().collect()),
            "all" => {
                cache.schedule_account_refresh();
                let mut values = Vec::new();
                for entry in ACCOUNT_KINDS {
                    if let Some(value) = cache.read(entry)? {
                        values.push(value);
                    }
                }
                Ok(values)
            }
            _ => Err(AicliCommandError::invalid_input(
                "kind must be one of account, groups, pricing, all",
            )),
        }
    }

    // -- Catalog and routes ------------------------------------------------------

    pub fn get_catalog(&self) -> Result<Option<Catalog>, AicliCommandError> {
        self.rebuild()?;
        if let Some(cache) = self
            .inner
            .lock()
            .map_err(|_| AicliCommandError::database())?
            .cache
            .clone()
        {
            let _ = cache.get("groups")?;
            let _ = cache.get("pricing")?;
        }
        Catalog::from_cache(&self.read_cache()).map_err(AicliCommandError::from)
    }

    pub async fn get_routes(&self) -> Result<AicliRoutesView, AicliCommandError> {
        let runtime = self
            .app_state
            .proxy_service
            .aicli_runtime()
            .await
            .unwrap_or_else(|| AicliRuntime::new(self.db.clone()));
        let snapshot = runtime.cached_projection()?;
        Ok(AicliRoutesView::from_snapshot(&snapshot))
    }

    // -- Lines and probes ---------------------------------------------------------

    pub fn list_lines(&self) -> Result<AicliLinesView, AicliCommandError> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|_| AicliCommandError::database())?;
        let selected_line_id = conn
            .query_row(
                "SELECT selected_line_id FROM aicli_account WHERE id=1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|_| AicliCommandError::database())?
            .unwrap_or_else(|| "main".to_owned());
        let mut lines = Vec::new();
        for entry in LINES.iter() {
            let health = Self::health_view(&conn, entry.id)?;
            lines.push(AicliLineView {
                id: entry.id.to_owned(),
                label_key: entry.label_key.to_owned(),
                transport: entry.transport.to_owned(),
                priority: entry.priority,
                first_byte_s: entry.first_byte_s,
                idle_s: entry.idle_s,
                stream_total_s: entry.stream_total_s,
                non_stream_s: entry.non_stream_s,
                health,
            });
        }
        Ok(AicliLinesView {
            selected_line_id,
            lines,
        })
    }

    fn health_view(
        conn: &Connection,
        line_id: &str,
    ) -> Result<Option<AicliLineHealthView>, AicliCommandError> {
        let Some((checked_at, status, latency_ms, http_status, error_code)) = conn
            .query_row(
                "SELECT checked_at,status,latency_ms,http_status,error_code
                 FROM aicli_line_health WHERE line_id=?1 ORDER BY checked_at DESC LIMIT 1",
                [line_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| AicliCommandError::database())?
        else {
            return Ok(None);
        };
        let next_check_at = if status == "fail" {
            Some(checked_at + backoff_seconds(consecutive_failures(conn, line_id)?))
        } else {
            None
        };
        Ok(Some(AicliLineHealthView {
            checked_at,
            status,
            latency_ms,
            http_status,
            error_code,
            next_check_at,
        }))
    }

    pub fn select_line(&self, line_id: String) -> Result<(), AicliCommandError> {
        if line(&line_id).is_none() {
            return Err(NewApiError::UnknownLine.into());
        }
        let changed = self
            .db
            .conn
            .lock()
            .map_err(|_| AicliCommandError::database())?
            .execute(
                "UPDATE aicli_account SET selected_line_id=?1 WHERE id=1",
                [&line_id],
            )
            .map_err(|_| AicliCommandError::database())?;
        if changed == 0 {
            return Err(AicliCommandError::not_bound());
        }
        // C3-11 修正：换线路改变绑定，旧聚合的在途写入按新代数丢弃。
        crate::aicli::cache::bump_binding_generation();
        self.rebuild()?;
        spawn_runtime_refresh(&self.app_state);
        Ok(())
    }

    pub async fn probe_lines(&self) -> Result<Vec<AicliProbeView>, AicliCommandError> {
        let now = Utc::now().timestamp();
        let probes: Vec<_> = LINES
            .iter()
            .map(|entry| self.probe_line(entry.id, now))
            .collect();
        futures::future::join_all(probes)
            .await
            .into_iter()
            .collect()
    }

    async fn probe_line(
        &self,
        line_id: &'static str,
        now: i64,
    ) -> Result<AicliProbeView, AicliCommandError> {
        let failures = {
            let conn = self
                .db
                .conn
                .lock()
                .map_err(|_| AicliCommandError::database())?;
            let latest: Option<(i64, String)> = conn
                .query_row(
                    "SELECT checked_at,status FROM aicli_line_health WHERE line_id=?1
                     ORDER BY checked_at DESC LIMIT 1",
                    [line_id],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(|_| AicliCommandError::database())?;
            match &latest {
                Some((checked_at, status)) if status == "fail" => {
                    let next = checked_at + backoff_seconds(consecutive_failures(&conn, line_id)?);
                    if now < next {
                        return Ok(AicliProbeView {
                            line_id: line_id.to_owned(),
                            status: "skipped".into(),
                            checked_at: *checked_at,
                            latency_ms: None,
                            http_status: None,
                            error_code: None,
                            next_check_at: Some(next),
                        });
                    }
                }
                _ => {}
            }
            consecutive_failures(&conn, line_id)?
        };
        let started = std::time::Instant::now();
        let outcome = (self.probe)(line_id).await;
        let latency_ms = started.elapsed().as_millis().min(i64::MAX as u128) as i64;
        let checked_at = Utc::now().timestamp();
        let (status, http_status, error_code) = match outcome {
            Ok((code, _)) => ("ok".to_owned(), Some(code as i64), None),
            Err(error) => {
                let http_status = match &error {
                    NewApiError::Http(code) => Some(*code as i64),
                    _ => None,
                };
                (
                    "fail".to_owned(),
                    http_status,
                    Some(probe_error_code(&error).to_owned()),
                )
            }
        };
        let next_check_at = (status == "fail").then(|| checked_at + backoff_seconds(failures + 1));
        self.db
            .conn
            .lock()
            .map_err(|_| AicliCommandError::database())?
            .execute(
                "INSERT INTO aicli_line_health(line_id,checked_at,status,latency_ms,http_status,error_code)
                 VALUES(?1,?2,?3,?4,?5,?6)
                 ON CONFLICT(line_id,checked_at) DO UPDATE SET status=excluded.status,
                 latency_ms=excluded.latency_ms,http_status=excluded.http_status,
                 error_code=excluded.error_code",
                rusqlite::params![
                    line_id,
                    checked_at,
                    status,
                    latency_ms,
                    http_status,
                    error_code
                ],
            )
            .map_err(|_| AicliCommandError::database())?;
        Ok(AicliProbeView {
            line_id: line_id.to_owned(),
            status,
            checked_at,
            latency_ms: Some(latency_ms),
            http_status,
            error_code,
            next_check_at,
        })
    }

    // -- Profiles -----------------------------------------------------------------

    pub fn get_profiles(&self) -> Result<AicliProfilesView, AicliCommandError> {
        let profiles = ProfileStore::new(self.db.clone()).list()?;
        let cache = self.read_cache();
        let presets = Catalog::from_cache(&cache)?
            .map(|catalog| {
                ["recommended", "low_cost"]
                    .iter()
                    .filter_map(|id| profiles::preset(id, &catalog).ok())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|_| AicliCommandError::database())?;
        let bindings = conn
            .prepare("SELECT scope,scope_key,profile_id FROM aicli_scope_binding ORDER BY scope,scope_key")
            .map_err(|_| AicliCommandError::database())?
            .query_map([], |row| {
                Ok(AicliScopeBindingView {
                    scope: row.get(0)?,
                    scope_key: row.get(1)?,
                    profile_id: row.get(2)?,
                })
            })
            .map_err(|_| AicliCommandError::database())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| AicliCommandError::database())?;
        let active_profile_id = conn
            .query_row(
                "SELECT active_profile_id FROM aicli_account WHERE id=1",
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|_| AicliCommandError::database())?
            .flatten();
        Ok(AicliProfilesView {
            profiles,
            presets,
            bindings,
            active_profile_id,
        })
    }

    pub fn save_profile(&self, profile: Profile) -> Result<Profile, AicliCommandError> {
        let cache = self.read_cache();
        let catalog = Catalog::from_cache(&cache)?.ok_or_else(|| {
            AicliCommandError::invalid_input("The model catalog has not synced yet")
        })?;
        let saved = ProfileStore::new(self.db.clone()).save(&profile, &catalog)?;
        spawn_runtime_refresh(&self.app_state);
        Ok(saved)
    }

    pub fn delete_profile(&self, id: String, revision: i64) -> Result<(), AicliCommandError> {
        ProfileStore::new(self.db.clone()).delete(&id, revision)?;
        spawn_runtime_refresh(&self.app_state);
        Ok(())
    }

    /// Owner decision 2026-11-04: scope is only `app` or `global`; `session`
    /// and `project` stay internal, `active_profile_id` has no v1 write path.
    pub fn set_active_profile(
        &self,
        scope: String,
        scope_key: String,
        id: Option<String>,
    ) -> Result<(), AicliCommandError> {
        let normalized_key = match scope.as_str() {
            "app" => AppType::from_str(scope_key.trim())
                .map_err(|error| AicliCommandError::invalid_input(error.to_string()))?
                .as_str()
                .to_owned(),
            "global" => {
                if !scope_key.trim().is_empty() {
                    return Err(AicliCommandError::invalid_input(
                        "The global binding takes no scope key",
                    ));
                }
                String::new()
            }
            other => {
                return Err(AicliCommandError::invalid_input(format!(
                    "scope must be app or global, not {other}"
                )))
            }
        };
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|_| AicliCommandError::database())?;
        match id {
            Some(id) => {
                let known = conn
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM aicli_profile WHERE id=?1)
                         OR ?1 IN ('recommended','low_cost')",
                        [&id],
                        |row| row.get::<_, bool>(0),
                    )
                    .map_err(|_| AicliCommandError::database())?;
                if !known {
                    return Err(AicliCommandError::invalid_input(format!(
                        "unknown profile: {id}"
                    )));
                }
                conn.execute(
                    "INSERT INTO aicli_scope_binding(scope,scope_key,profile_id) VALUES(?1,?2,?3)
                     ON CONFLICT(scope,scope_key) DO UPDATE SET profile_id=excluded.profile_id",
                    rusqlite::params![scope, normalized_key, id],
                )
                .map_err(|_| AicliCommandError::database())?;
            }
            None => {
                conn.execute(
                    "DELETE FROM aicli_scope_binding WHERE scope=?1 AND scope_key=?2",
                    rusqlite::params![scope, normalized_key],
                )
                .map_err(|_| AicliCommandError::database())?;
            }
        }
        drop(conn);
        spawn_runtime_refresh(&self.app_state);
        Ok(())
    }

    // -- Takeover -------------------------------------------------------------------

    pub fn takeover_status(
        &self,
    ) -> Result<Vec<crate::aicli::takeover::TakeoverStatus>, AicliCommandError> {
        takeover::status(&self.app_state).map_err(AicliCommandError::from)
    }

    pub async fn take_over(
        &self,
        app_id: String,
        confirmed: bool,
    ) -> Result<(), AicliCommandError> {
        let app = AppType::from_str(app_id.trim())
            .map_err(|error| AicliCommandError::invalid_input(error.to_string()))?;
        takeover::take_over(&self.app_state, self.secrets.as_ref(), &app, confirmed)
            .await
            .map_err(AicliCommandError::from)
    }

    pub async fn release_takeover(&self, app_id: String) -> Result<(), AicliCommandError> {
        let app = AppType::from_str(app_id.trim())
            .map_err(|error| AicliCommandError::invalid_input(error.to_string()))?;
        takeover::release(&self.app_state, self.secrets.as_ref(), &app)
            .await
            .map_err(AicliCommandError::from)
    }

    // -- Usage and bills ---------------------------------------------------------------

    pub fn get_usage_summary(&self) -> Result<AicliUsageSummaryView, AicliCommandError> {
        self.rebuild()?;
        if let Some(cache) = self
            .inner
            .lock()
            .map_err(|_| AicliCommandError::database())?
            .cache
            .clone()
        {
            // C1-06 freshness: today's row refreshes on read, past days are final.
            let today = Utc::now().format("%Y-%m-%d").to_string();
            let _ = cache.get_usage(&today)?;
        }
        let now = Utc::now().timestamp();
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|_| AicliCommandError::database())?;
        let days = conn
            .prepare(
                "SELECT day,quota,requests,is_final,fetched_at FROM aicli_usage_daily ORDER BY day DESC",
            )
            .map_err(|_| AicliCommandError::database())?
            .query_map([], |row| {
                Ok(AicliUsageDayView {
                    day: row.get(0)?,
                    quota: row.get(1)?,
                    requests: row.get(2)?,
                    is_final: row.get(3)?,
                    fetched_at: row.get(4)?,
                    stale: !row.get::<_, bool>(3)? && now.saturating_sub(row.get(4)?) >= 600,
                })
            })
            .map_err(|_| AicliCommandError::database())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| AicliCommandError::database())?;
        let total_quota = days.iter().map(|day| day.quota).sum();
        let total_requests = days.iter().map(|day| day.requests).sum();
        Ok(AicliUsageSummaryView {
            days,
            total_quota,
            total_requests,
        })
    }

    pub fn get_request_bill(
        &self,
        request_id: String,
    ) -> Result<Option<AicliRequestBillView>, AicliCommandError> {
        self.db
            .conn
            .lock()
            .map_err(|_| AicliCommandError::database())?
            .query_row(
                "SELECT request_id,created_at,app_type,model,status_code,aicli_line_id,
                        aicli_profile_id,upstream_request_id,server_group,billed_quota,billed_at
                 FROM proxy_request_logs WHERE request_id=?1",
                [&request_id],
                |row| {
                    Ok(AicliRequestBillView {
                        request_id: row.get(0)?,
                        created_at: row.get(1)?,
                        app_type: row.get(2)?,
                        model: row.get(3)?,
                        status_code: row.get(4)?,
                        line_id: row.get(5)?,
                        profile_id: row.get(6)?,
                        upstream_request_id: row.get(7)?,
                        server_group: row.get(8)?,
                        billed_quota: row.get(9)?,
                        billed_at: row.get(10)?,
                    })
                },
            )
            .optional()
            .map_err(|_| AicliCommandError::database())
    }

    // -- Privacy ---------------------------------------------------------------------

    pub fn get_privacy(&self) -> Result<AicliPrivacyView, AicliCommandError> {
        let row = self
            .db
            .conn
            .lock()
            .map_err(|_| AicliCommandError::database())?
            .query_row(
                "SELECT privacy_enabled,privacy_failure_mode,allow_unscanned_media
                 FROM aicli_account WHERE id=1",
                [],
                |row| {
                    Ok((
                        row.get::<_, bool>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, bool>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| AicliCommandError::database())?;
        Ok(match row {
            Some((privacy_enabled, privacy_failure_mode, allow_unscanned_media)) => {
                AicliPrivacyView {
                    bound: true,
                    privacy_enabled,
                    privacy_failure_mode,
                    allow_unscanned_media,
                }
            }
            None => AicliPrivacyView {
                bound: false,
                privacy_enabled: false,
                privacy_failure_mode: "block".into(),
                allow_unscanned_media: false,
            },
        })
    }

    pub fn set_privacy(
        &self,
        privacy_enabled: bool,
        privacy_failure_mode: String,
        allow_unscanned_media: bool,
    ) -> Result<(), AicliCommandError> {
        if !matches!(privacy_failure_mode.as_str(), "block" | "pass") {
            return Err(AicliCommandError::invalid_input(
                "privacy_failure_mode must be block or pass",
            ));
        }
        let changed = self
            .db
            .conn
            .lock()
            .map_err(|_| AicliCommandError::database())?
            .execute(
                "UPDATE aicli_account SET privacy_enabled=?1,privacy_failure_mode=?2,
                 allow_unscanned_media=?3 WHERE id=1",
                rusqlite::params![privacy_enabled, privacy_failure_mode, allow_unscanned_media],
            )
            .map_err(|_| AicliCommandError::database())?;
        if changed == 0 {
            return Err(AicliCommandError::not_bound());
        }
        // E-06：隐私设置已变，代理内的内存缓存按修订号失效（无需同一运行时）。
        crate::aicli::privacy::layer::bump_revision();
        Ok(())
    }
}

impl AicliRoutesView {
    fn from_snapshot(snapshot: &RoutesSnapshot) -> Self {
        Self {
            fingerprint: snapshot.fingerprint.clone(),
            active_profile_id: snapshot.active_profile_id.clone(),
            selected_line_id: snapshot.selected_line_id.clone(),
            profiles: snapshot
                .profiles
                .values()
                .map(|entry| AicliRouteProfileView {
                    id: entry.profile.id.clone(),
                    name: entry.profile.name.clone(),
                    source: entry.profile.source.clone(),
                    preset_id: entry.profile.preset_id.clone(),
                    revision: entry.profile.revision,
                    routes: entry.compiled.routes.clone(),
                    repair: entry.compiled.repair.clone(),
                })
                .collect(),
        }
    }
}

fn release_lifecycle_callback(
    app_state: &AppState,
    secrets: Arc<dyn SecretStore>,
) -> Arc<ReleaseAll> {
    let app_state = app_state.clone();
    Arc::new(move || {
        let app_state = app_state.clone();
        let secrets = secrets.clone();
        Box::pin(release_all_takeovers(app_state, secrets))
    })
}

/// Rebuild the proxy runtime projection after local mutations; the background
/// loop would converge too, but profile and line changes should apply now.
fn spawn_runtime_refresh(app_state: &AppState) {
    let app_state = app_state.clone();
    tauri::async_runtime::spawn(async move {
        if let Some(runtime) = app_state.proxy_service.aicli_runtime().await {
            if let Err(error) = runtime.refresh().await {
                log::debug!("AICLI runtime refresh after a local mutation failed: {error}");
            }
        }
    });
}

/// Managed Tauri state holding the aggregate.
pub struct AicliState(pub Arc<AicliServices>);

#[tauri::command]
pub async fn aicli_get_account(
    state: State<'_, AicliState>,
) -> Result<AicliAccountView, AicliCommandError> {
    state.0.get_account()
}

#[tauri::command]
pub async fn aicli_bind(
    state: State<'_, AicliState>,
    access_token: String,
    line: String,
) -> Result<AicliBindResult, AicliCommandError> {
    state.0.bind(access_token, line).await
}

#[tauri::command]
pub async fn aicli_unbind(
    state: State<'_, AicliState>,
    delete_created_tokens: bool,
) -> Result<(), AicliCommandError> {
    state.0.unbind(delete_created_tokens).await
}

#[tauri::command]
pub async fn aicli_refresh(
    state: State<'_, AicliState>,
    kind: String,
) -> Result<Vec<CachedValue>, AicliCommandError> {
    state.0.refresh(kind)
}

#[tauri::command]
pub async fn aicli_get_catalog(
    state: State<'_, AicliState>,
) -> Result<Option<Catalog>, AicliCommandError> {
    state.0.get_catalog()
}

#[tauri::command]
pub async fn aicli_get_routes(
    state: State<'_, AicliState>,
) -> Result<AicliRoutesView, AicliCommandError> {
    state.0.get_routes().await
}

#[tauri::command]
pub async fn aicli_list_lines(
    state: State<'_, AicliState>,
) -> Result<AicliLinesView, AicliCommandError> {
    state.0.list_lines()
}

#[tauri::command]
pub async fn aicli_select_line(
    state: State<'_, AicliState>,
    id: String,
) -> Result<(), AicliCommandError> {
    state.0.select_line(id)
}

#[tauri::command]
pub async fn aicli_probe_lines(
    state: State<'_, AicliState>,
) -> Result<Vec<AicliProbeView>, AicliCommandError> {
    state.0.probe_lines().await
}

#[tauri::command]
pub async fn aicli_get_profiles(
    state: State<'_, AicliState>,
) -> Result<AicliProfilesView, AicliCommandError> {
    state.0.get_profiles()
}

#[tauri::command]
pub async fn aicli_save_profile(
    state: State<'_, AicliState>,
    profile: Profile,
) -> Result<Profile, AicliCommandError> {
    state.0.save_profile(profile)
}

#[tauri::command]
pub async fn aicli_delete_profile(
    state: State<'_, AicliState>,
    id: String,
    revision: i64,
) -> Result<(), AicliCommandError> {
    state.0.delete_profile(id, revision)
}

#[tauri::command]
pub async fn aicli_set_active_profile(
    state: State<'_, AicliState>,
    scope: String,
    scope_key: String,
    id: Option<String>,
) -> Result<(), AicliCommandError> {
    state.0.set_active_profile(scope, scope_key, id)
}

#[tauri::command]
pub async fn aicli_takeover_status(
    state: State<'_, AicliState>,
) -> Result<Vec<crate::aicli::takeover::TakeoverStatus>, AicliCommandError> {
    state.0.takeover_status()
}

#[tauri::command]
pub async fn aicli_take_over(
    state: State<'_, AicliState>,
    app: String,
    confirmed: bool,
) -> Result<(), AicliCommandError> {
    state.0.take_over(app, confirmed).await
}

#[tauri::command]
pub async fn aicli_release(
    state: State<'_, AicliState>,
    app: String,
) -> Result<(), AicliCommandError> {
    state.0.release_takeover(app).await
}

#[tauri::command]
pub async fn aicli_get_usage_summary(
    state: State<'_, AicliState>,
) -> Result<AicliUsageSummaryView, AicliCommandError> {
    state.0.get_usage_summary()
}

#[tauri::command]
pub async fn aicli_get_request_bill(
    state: State<'_, AicliState>,
    request_id: String,
) -> Result<Option<AicliRequestBillView>, AicliCommandError> {
    state.0.get_request_bill(request_id)
}

#[tauri::command]
pub async fn aicli_get_privacy(
    state: State<'_, AicliState>,
) -> Result<AicliPrivacyView, AicliCommandError> {
    state.0.get_privacy()
}

#[tauri::command]
pub async fn aicli_set_privacy(
    state: State<'_, AicliState>,
    privacy_enabled: bool,
    privacy_failure_mode: String,
    allow_unscanned_media: bool,
) -> Result<(), AicliCommandError> {
    state
        .0
        .set_privacy(privacy_enabled, privacy_failure_mode, allow_unscanned_media)
}

#[cfg(test)]
mod tests;
