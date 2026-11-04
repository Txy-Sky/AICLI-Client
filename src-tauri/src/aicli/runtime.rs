//! Cache-first runtime. Snapshots contain public routing facts, never credentials.
use std::collections::{BTreeMap, HashMap};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use lru::LruCache;
use rusqlite::{params, OptionalExtension};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::catalog::Catalog;
use super::newapi::{NewApiClient, NewApiError};
use super::profiles::{preset, CompiledProfile, Profile, ProfileError, ProfileStore};
use super::secrets::{KeyringStore, SecretError, SecretStore};
use super::tokens::{groups_hash, TokenError, TokenRemote, TokenService};
use crate::Database;

const SESSION_CAPACITY: usize = 4096;
const SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Default)]
pub struct Cooldown {
    // Storage only here; failure classification and backoff belong to C3-05/06.
    pub lines: Mutex<HashMap<String, Instant>>,
    pub models: Mutex<HashMap<(String, String), Instant>>,
}

#[derive(Debug, Clone)]
pub struct ProfileRoutes {
    pub profile: Profile,
    pub compiled: CompiledProfile,
    pub token_ids: BTreeMap<String, i64>,
}

#[derive(Debug, Clone, Default)]
pub struct RoutesSnapshot {
    pub fingerprint: String,
    pub catalog: Option<Catalog>,
    pub profiles: BTreeMap<String, ProfileRoutes>,
    pub active_profile_id: Option<String>,
    pub selected_line_id: String,
}

#[derive(Clone)]
pub struct SessionPin {
    pub profile_id: String,
    pub snapshot: Arc<RoutesSnapshot>,
    created_at: Instant,
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("Runtime storage is unavailable")]
    Storage,
    #[error(transparent)]
    Api(#[from] NewApiError),
    #[error(transparent)]
    Profile(#[from] ProfileError),
    #[error(transparent)]
    Secret(#[from] SecretError),
    #[error(transparent)]
    Token(#[from] TokenError),
}

pub type RuntimeValues = (Value, Value, Value, Value);
type Binding = (i64, String, Option<String>, String);
type BuiltRoutes = (RoutesSnapshot, Vec<Vec<String>>);

struct BoundTokenService {
    user_id: i64,
    line: String,
    access: String,
    service: Arc<TokenService>,
}

/// The response values are public account/catalog facts. Credentials are supplied
/// from SecretStore at operation time, so a rebind cannot retain an old client.
pub trait RuntimeRemote: Send + Sync {
    fn fetch<'a>(
        &'a self,
        api: &'a NewApiClient,
    ) -> BoxFuture<'a, Result<RuntimeValues, NewApiError>>;
    fn tokens(&self, api: NewApiClient) -> Arc<dyn TokenRemote>;
}

struct HttpRemote;
impl RuntimeRemote for HttpRemote {
    fn fetch<'a>(
        &'a self,
        api: &'a NewApiClient,
    ) -> BoxFuture<'a, Result<RuntimeValues, NewApiError>> {
        Box::pin(async move {
            let account = api.user_self().await?;
            let groups = api.user_groups().await?;
            let pricing = api.pricing().await?;
            let auto = api.auto_groups_setting().await?;
            Ok((account, groups, pricing, auto))
        })
    }
    fn tokens(&self, api: NewApiClient) -> Arc<dyn TokenRemote> {
        Arc::new(api)
    }
}

#[cfg(any(test, feature = "test-hooks"))]
type LocalProviderSource =
    dyn Fn(&crate::AppType, &str, &str) -> Option<crate::provider::Provider> + Send + Sync;

pub struct AicliRuntime {
    #[cfg(any(test, feature = "test-hooks"))]
    local_provider_source: RwLock<Option<Arc<LocalProviderSource>>>,
    store: Arc<dyn SecretStore>,
    db: Arc<Database>,
    remote: Arc<dyn RuntimeRemote>,
    routes: RwLock<Arc<RoutesSnapshot>>,
    credentials: RwLock<HashMap<i64, String>>,
    session_pins: Mutex<LruCache<String, SessionPin>>,
    pub cooldown: Cooldown,
    refresh_lock: tokio::sync::Mutex<()>,
    background_started: std::sync::atomic::AtomicBool,
    token_service: Mutex<Option<BoundTokenService>>,
    identity: Mutex<Option<(i64, String)>>,
}

impl AicliRuntime {
    pub fn new(db: Arc<Database>) -> Arc<Self> {
        Self::with_remote(db, Arc::new(KeyringStore::default()), Arc::new(HttpRemote))
    }

    pub fn with_remote(
        db: Arc<Database>,
        store: Arc<dyn SecretStore>,
        remote: Arc<dyn RuntimeRemote>,
    ) -> Arc<Self> {
        let runtime = Arc::new(Self {
            #[cfg(any(test, feature = "test-hooks"))]
            local_provider_source: RwLock::new(None),
            db,
            store,
            remote,
            routes: RwLock::new(Arc::new(RoutesSnapshot::default())),
            credentials: RwLock::new(HashMap::new()),
            session_pins: Mutex::new(LruCache::new(NonZeroUsize::new(SESSION_CAPACITY).unwrap())),
            cooldown: Cooldown::default(),
            refresh_lock: Default::default(),
            background_started: Default::default(),
            token_service: Default::default(),
            identity: Default::default(),
        });
        // No network, task spawn, or block_on during construction.
        if runtime.load_cached().is_err() {
            log::warn!("AICLI cached routes unavailable; starting with empty routes");
        }
        runtime
    }

    /// Isolated forwarding fixtures may substitute loopback Provider DTOs only.
    /// Account/profile/session resolution still executes the production resolver.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn set_local_test_provider_source(
        &self,
        source: impl Fn(&crate::AppType, &str, &str) -> Option<crate::provider::Provider>
            + Send
            + Sync
            + 'static,
    ) {
        *self.local_provider_source.write().unwrap() = Some(Arc::new(source));
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn local_test_provider(
        &self,
        app: &crate::AppType,
        line: &str,
        wire: &str,
    ) -> Result<Option<crate::provider::Provider>, RuntimeError> {
        let Some(provider) = self
            .local_provider_source
            .read()
            .map_err(|_| RuntimeError::Storage)?
            .as_ref()
            .and_then(|source| source(app, line, wire))
        else {
            return Ok(None);
        };
        let base = crate::proxy::providers::get_adapter(app)
            .ok_or(RuntimeError::Storage)?
            .extract_base_url(&provider)
            .map_err(|_| RuntimeError::Storage)?;
        let url = url::Url::parse(&base).map_err(|_| RuntimeError::Storage)?;
        if url.scheme() != "http"
            || !matches!(url.host_str(), Some("127.0.0.1" | "localhost"))
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(RuntimeError::Storage);
        }
        Ok(Some(provider))
    }

    pub fn snapshot(&self) -> Arc<RoutesSnapshot> {
        self.routes
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn credential(&self, token_id: i64) -> Option<String> {
        self.credentials.read().ok()?.get(&token_id).cloned()
    }

    pub(crate) fn database(&self) -> &Database {
        &self.db
    }

    /// Do not serve an old account snapshot after unbind/rebind, even before
    /// the background refresh has had a chance to clear it.
    pub(crate) fn bound_for_routing(&self) -> Result<bool, RuntimeError> {
        let Some((id, _, _, access)) = self.binding()? else {
            return Ok(false);
        };
        Ok(self
            .identity
            .lock()
            .map_err(|_| RuntimeError::Storage)?
            .as_ref()
            .is_some_and(|(old_id, old_access)| *old_id == id && *old_access == access))
    }

    pub fn pin(&self, key: String, profile_id: String, snapshot: Arc<RoutesSnapshot>) {
        self.session_pins
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .put(
                key,
                SessionPin {
                    profile_id,
                    snapshot,
                    created_at: Instant::now(),
                },
            );
    }

    pub fn session_pin(&self, key: &str) -> Option<SessionPin> {
        let mut pins = self.session_pins.lock().unwrap_or_else(|e| e.into_inner());
        if pins
            .peek(key)
            .is_some_and(|p| p.created_at.elapsed() >= SESSION_TTL)
        {
            pins.pop(key);
            return None;
        }
        pins.get(key).cloned()
    }

    fn binding(&self) -> Result<Option<Binding>, RuntimeError> {
        let row = self
            .db
            .conn
            .lock()
            .map_err(|_| RuntimeError::Storage)?
            .query_row(
                "SELECT user_id,selected_line_id,active_profile_id FROM aicli_account WHERE id=1",
                [],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| RuntimeError::Storage)?;
        let Some((id, line, active)) = row else {
            return Ok(None);
        };
        let Some(access) = self.store.get("access-token")?.filter(|s| !s.is_empty()) else {
            return Ok(None);
        };
        Ok(Some((id, line, active, access)))
    }

    fn cache(&self, kind: &str) -> Result<Option<Value>, RuntimeError> {
        let body: Option<String> = self
            .db
            .conn
            .lock()
            .map_err(|_| RuntimeError::Storage)?
            .query_row(
                "SELECT body_json FROM aicli_cache WHERE kind=?1",
                [kind],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| RuntimeError::Storage)?;
        body.map(|s| serde_json::from_str(&s).map_err(|_| RuntimeError::Storage))
            .transpose()
    }

    fn clear(&self) {
        *self.routes.write().unwrap_or_else(|e| e.into_inner()) =
            Arc::new(RoutesSnapshot::default());
        self.credentials
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.session_pins
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        *self.token_service.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.identity.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    fn build(
        &self,
        groups: &Value,
        pricing: &Value,
        auto: Option<&Value>,
        line: String,
        active: Option<String>,
    ) -> Result<BuiltRoutes, RuntimeError> {
        let mut pricing = pricing.clone();
        if let Some(groups) = auto.and_then(|v| v.pointer("/data/groups")) {
            pricing["auto_groups"] = groups.clone();
        }
        let catalog = Catalog::from_snapshots(groups, &pricing)?;
        let max = auto
            .and_then(|v| v.pointer("/data/max_count"))
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(usize::MAX);
        let store = ProfileStore::new(self.db.clone());
        let mut profiles = store.list()?;
        profiles.push(preset("recommended", &catalog)?);
        profiles.push(preset("low_cost", &catalog)?);
        let mut canonical = serde_json::json!({"catalog":catalog,"profiles":profiles,"max":max,"line":line,"active":active});
        canonical.sort_all_objects();
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&canonical).map_err(|_| RuntimeError::Storage)?)
        );
        let mut snapshot = RoutesSnapshot {
            fingerprint,
            catalog: Some(catalog.clone()),
            profiles: BTreeMap::new(),
            active_profile_id: active,
            selected_line_id: line,
        };
        let mut references = Vec::new();
        for profile in profiles {
            let compiled = store.compile(&profile, &catalog, max)?;
            references.extend(compiled.routes.iter().map(|r| r.ordered_groups.clone()));
            if let Some(baseline) = store.baseline(&profile.id)? {
                references.extend(baseline.routes.into_values());
            }
            snapshot.profiles.insert(
                profile.id.clone(),
                ProfileRoutes {
                    profile,
                    compiled,
                    token_ids: BTreeMap::new(),
                },
            );
        }
        Ok((snapshot, references))
    }

    fn cached_credentials(
        &self,
        snapshot: &mut RoutesSnapshot,
    ) -> Result<HashMap<i64, String>, RuntimeError> {
        let records = {
            let conn = self.db.conn.lock().map_err(|_| RuntimeError::Storage)?;
            let mut s = conn
                .prepare("SELECT groups_hash,token_id FROM aicli_managed_token")
                .map_err(|_| RuntimeError::Storage)?;
            let rows = s
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
                .map_err(|_| RuntimeError::Storage)?;
            rows.collect::<Result<BTreeMap<_, _>, _>>()
                .map_err(|_| RuntimeError::Storage)?
        };
        let mut credentials = HashMap::new();
        for entry in snapshot.profiles.values_mut() {
            for route in &entry.compiled.routes {
                if let Some(id) = records.get(&groups_hash(&route.ordered_groups)) {
                    if let Some(key) = self
                        .store
                        .get(&format!("token:{id}"))?
                        .filter(|s| !s.is_empty())
                    {
                        entry.token_ids.insert(route.public_model.clone(), *id);
                        credentials.insert(*id, key);
                    }
                }
            }
        }
        Ok(credentials)
    }

    fn load_cached(&self) -> Result<(), RuntimeError> {
        let Some((id, line, active, access)) = self.binding()? else {
            self.clear();
            return Ok(());
        };
        *self.identity.lock().map_err(|_| RuntimeError::Storage)? = Some((id, access));
        let (Some(groups), Some(pricing)) = (self.cache("groups")?, self.cache("pricing")?) else {
            return Ok(());
        };
        let (mut snapshot, _) = self.build(
            &groups,
            &pricing,
            self.cache("auto_groups")?.as_ref(),
            line,
            active,
        )?;
        let credentials = self.cached_credentials(&mut snapshot)?;
        *self
            .credentials
            .write()
            .map_err(|_| RuntimeError::Storage)? = credentials;
        *self.routes.write().map_err(|_| RuntimeError::Storage)? = Arc::new(snapshot);
        Ok(())
    }

    /// A failed fetch, compile or credential operation never publishes half a snapshot.
    pub async fn refresh(&self) -> Result<(), RuntimeError> {
        let _guard = self.refresh_lock.lock().await;
        let Some(binding) = self.binding()? else {
            self.clear();
            return Ok(());
        };
        let (id, line, active, access) = &binding;
        let same_account = self
            .identity
            .lock()
            .map_err(|_| RuntimeError::Storage)?
            .as_ref()
            .is_some_and(|(old_id, old_access)| old_id == id && old_access == access);
        if !same_account {
            self.clear();
            self.load_cached()?;
        }
        let api = NewApiClient::new(access, line)?;
        let (account, groups, pricing, auto) = self.remote.fetch(&api).await?;
        if account.pointer("/data/id").and_then(Value::as_i64) != Some(*id) {
            return Err(NewApiError::InvalidResponse.into());
        }
        if self.binding()?.as_ref() != Some(&binding) {
            self.clear();
            self.load_cached()?;
            return Ok(());
        }
        let (mut next, references) =
            self.build(&groups, &pricing, Some(&auto), line.clone(), active.clone())?;
        let unchanged = next.fingerprint == self.snapshot().fingerprint;
        let mut credentials = self.cached_credentials(&mut next)?;
        let missing = next
            .profiles
            .values()
            .any(|p| p.compiled.routes.len() != p.token_ids.len());
        if !unchanged || missing {
            let orders: Vec<_> = next
                .profiles
                .values()
                .flat_map(|p| p.compiled.routes.iter().map(|r| r.ordered_groups.clone()))
                .collect();
            let service = {
                let mut saved = self
                    .token_service
                    .lock()
                    .map_err(|_| RuntimeError::Storage)?;
                if saved
                    .as_ref()
                    .is_none_or(|s| s.user_id != *id || &s.line != line || &s.access != access)
                {
                    *saved = Some(BoundTokenService {
                        user_id: *id,
                        line: line.clone(),
                        access: access.clone(),
                        service: Arc::new(TokenService::new(
                            self.db.clone(),
                            self.store.clone(),
                            self.remote.tokens(api),
                        )),
                    });
                }
                saved.as_ref().ok_or(RuntimeError::Storage)?.service.clone()
            };
            let ids = service
                .ensure_compilation(
                    &orders,
                    next.catalog.as_ref().is_some_and(|c| c.auto_available),
                    &references,
                )
                .await?;
            for entry in next.profiles.values_mut() {
                for route in &entry.compiled.routes {
                    let token_id = *ids
                        .get(&groups_hash(&route.ordered_groups))
                        .ok_or(RuntimeError::Storage)?;
                    let key = self
                        .store
                        .get(&format!("token:{token_id}"))?
                        .ok_or(RuntimeError::Storage)?;
                    credentials.insert(token_id, key);
                    entry.token_ids.insert(route.public_model.clone(), token_id);
                }
            }
        }
        // A rebind/unbind while HTTP was in flight must never publish old account routes.
        let mut conn = self.db.conn.lock().map_err(|_| RuntimeError::Storage)?;
        let tx = conn.transaction().map_err(|_| RuntimeError::Storage)?;
        let current = tx
            .query_row(
                "SELECT user_id,selected_line_id,active_profile_id FROM aicli_account WHERE id=1",
                [],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| RuntimeError::Storage)?;
        let access_current = self.store.get("access-token")?;
        if current != Some((*id, line.clone(), active.clone()))
            || access_current.as_deref() != Some(access.as_str())
        {
            drop(tx);
            drop(conn);
            self.clear();
            self.load_cached()?;
            return Ok(());
        }
        for (kind, value) in [
            ("account", account),
            ("groups", groups),
            ("pricing", pricing),
            ("auto_groups", auto),
        ] {
            let body = serde_json::to_string(&value).map_err(|_| RuntimeError::Storage)?;
            let hash = format!("{:x}", Sha256::digest(body.as_bytes()));
            tx.execute("INSERT INTO aicli_cache(kind,fingerprint,fetched_at,body_json) VALUES(?1,?2,?3,?4) ON CONFLICT(kind) DO UPDATE SET fingerprint=excluded.fingerprint,fetched_at=excluded.fetched_at,body_json=excluded.body_json",params![kind,hash,chrono::Utc::now().timestamp(),body]).map_err(|_|RuntimeError::Storage)?;
        }
        tx.commit().map_err(|_| RuntimeError::Storage)?;
        // Keep older credentials in memory for snapshots held by existing session pins.
        self.credentials
            .write()
            .map_err(|_| RuntimeError::Storage)?
            .extend(credentials);
        if !unchanged || missing {
            *self.routes.write().map_err(|_| RuntimeError::Storage)? = Arc::new(next);
        }
        Ok(())
    }

    pub fn start_background(self: &Arc<Self>) {
        use std::sync::atomic::Ordering;
        if self.background_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let weak = Arc::downgrade(self);
        tauri::async_runtime::spawn(async move {
            loop {
                let Some(runtime) = weak.upgrade() else {
                    return;
                };
                if runtime.refresh().await.is_err() {
                    log::warn!("AICLI background refresh failed; retaining prior route snapshot");
                }
                drop(runtime);
                tokio::time::sleep(Duration::from_secs(600)).await;
            }
        });
    }
}

#[cfg(test)]
mod tests;
