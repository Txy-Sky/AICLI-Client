//! Exact client-file snapshots stay in SecretStore, never proxy_live_backup.
use std::{path::PathBuf, sync::OnceLock};

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use super::secrets::SecretStore;
use crate::{
    app_config::AppType,
    live::{
        engine::{digest, read_current, LiveFile},
        patch::{Guarded, WholeFile},
    },
    mode::{
        controller,
        operation::{AppWrite, FileChange},
        state::PendingTarget,
    },
    store::AppState,
};

// Detection only: these values must never be projected into a client file.
const LEGACY_CC_SWITCH_PORT: u16 = 15721;
const LEGACY_CC_SWITCH_DESKTOP_PROFILE_ID: &str = "00000000-0000-4000-8000-000000157210";

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TakeoverError {
    #[error("CC Switch is managing this client; confirmation is required")]
    CcSwitchConflict,
    #[error("No compatible models; select groups first")]
    NoModels,
    #[error("Desktop supports at most 200 models")]
    TooManyModels,
    #[error("Unsupported client")]
    Unsupported,
    #[error("Client backup is unavailable or invalid")]
    Backup,
    #[error("Client configuration operation failed")]
    Client,
    #[error("Takeover state could not be persisted")]
    State,
    #[error("Restored client files differ from the backup")]
    Verification,
}

#[derive(Debug, Serialize)]
pub struct TakeoverStatus {
    pub app: String,
    pub state: String,
    pub taken_over_at: Option<i64>,
}

#[derive(Serialize, Deserialize)]
struct Snapshot {
    version: u8,
    files: Vec<Option<Vec<u8>>>,
}

fn index(app: &AppType) -> Result<usize, TakeoverError> {
    controller::PROXY_APPS
        .iter()
        .position(|a| a == app)
        .ok_or(TakeoverError::Unsupported)
}
fn lock(app: &AppType) -> Result<&'static tokio::sync::Mutex<()>, TakeoverError> {
    static LOCKS: OnceLock<[tokio::sync::Mutex<()>; 5]> = OnceLock::new();
    Ok(&LOCKS.get_or_init(|| std::array::from_fn(|_| tokio::sync::Mutex::new(())))[index(app)?])
}
fn account(app: &AppType) -> String {
    format!("takeover-backup:{}", app.as_str())
}

fn files(app: &AppType) -> Result<Vec<PathBuf>, TakeoverError> {
    Ok(match app {
        AppType::Claude => vec![crate::config::get_claude_settings_path()],
        AppType::Codex => vec![
            crate::codex_config::get_codex_auth_path(),
            crate::codex_config::get_codex_config_path(),
            crate::codex_config::get_codex_model_catalog_path(),
            crate::codex_config::get_codex_managed_oauth_live_auth_marker_path(),
            crate::live::engine::DeviceStore::for_device().file("codex-login-stash.json"),
        ],
        AppType::Gemini => vec![
            crate::gemini_config::get_gemini_env_path(),
            crate::gemini_config::get_gemini_settings_path(),
        ],
        AppType::GrokBuild => vec![crate::grok_config::get_grok_config_path()],
        AppType::ClaudeDesktop => crate::claude_desktop_config::takeover_file_paths()
            .map_err(|_| TakeoverError::Client)?,
        _ => return Err(TakeoverError::Unsupported),
    })
}
fn snapshot(app: &AppType) -> Result<Snapshot, TakeoverError> {
    Ok(Snapshot {
        version: 1,
        files: files(app)?
            .iter()
            .map(|p| read_current(p).map_err(|_| TakeoverError::Client))
            .collect::<Result<_, _>>()?,
    })
}
fn conflict(app: &AppType, snapshot: &Snapshot) -> bool {
    let old_url = format!("http://127.0.0.1:{LEGACY_CC_SWITCH_PORT}");
    snapshot.files.iter().flatten().any(|bytes| {
        let text = String::from_utf8_lossy(bytes);
        text.contains(&old_url)
            || text.contains("PROXY_MANAGED")
            || (*app == AppType::ClaudeDesktop
                && text.contains(LEGACY_CC_SWITCH_DESKTOP_PROFILE_ID))
    })
}
pub fn status(state: &AppState) -> Result<Vec<TakeoverStatus>, TakeoverError> {
    let conn = state.db.conn.lock().map_err(|_| TakeoverError::State)?;
    let mut stmt = conn
        .prepare("SELECT app_type,state,taken_over_at FROM aicli_takeover ORDER BY app_type")
        .map_err(|_| TakeoverError::State)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(TakeoverStatus {
                app: r.get(0)?,
                state: r.get(1)?,
                taken_over_at: r.get(2)?,
            })
        })
        .map_err(|_| TakeoverError::State)?;
    rows.collect::<Result<_, _>>()
        .map_err(|_| TakeoverError::State)
}
fn current(state: &AppState, app: &AppType) -> Result<Option<String>, TakeoverError> {
    state
        .db
        .conn
        .lock()
        .map_err(|_| TakeoverError::State)?
        .query_row(
            "SELECT state FROM aicli_takeover WHERE app_type=?1",
            [app.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| TakeoverError::State)
}
fn set_state(state: &AppState, app: &AppType, value: &str) -> Result<(), TakeoverError> {
    state.db.conn.lock().map_err(|_| TakeoverError::State)?.execute("INSERT INTO aicli_takeover(app_type,state,backup_ref,taken_over_at) VALUES(?1,?2,?3,?4) ON CONFLICT(app_type) DO UPDATE SET state=excluded.state", rusqlite::params![app.as_str(), value, account(app), chrono::Utc::now().timestamp()]).map_err(|_| TakeoverError::State)?;
    Ok(())
}
fn finish_release(
    state: &AppState,
    store: &dyn SecretStore,
    app: &AppType,
) -> Result<(), TakeoverError> {
    // A durable completion marker makes a crash between the two deletions safe.
    set_state(state, app, "released")?;
    store
        .delete(&account(app))
        .map_err(|_| TakeoverError::Backup)?;
    state
        .db
        .conn
        .lock()
        .map_err(|_| TakeoverError::State)?
        .execute(
            "DELETE FROM aicli_takeover WHERE app_type=?1",
            [app.as_str()],
        )
        .map_err(|_| TakeoverError::State)?;
    Ok(())
}

pub async fn take_over(
    state: &AppState,
    store: &dyn SecretStore,
    app: &AppType,
    confirmed: bool,
) -> Result<(), TakeoverError> {
    let _guard = lock(app)?.lock().await;
    let runtime = state
        .proxy_service
        .aicli_runtime()
        .await
        .unwrap_or_else(|| super::runtime::AicliRuntime::new(state.db.clone()));
    let seed = state
        .db
        .get_provider_by_id("aicli-main", app.as_str())
        .map_err(|_| TakeoverError::State)?
        .ok_or(TakeoverError::State)?;
    let projection = runtime
        .cached_projection()
        .map_err(|_| TakeoverError::State)?;
    prepare_projection(&state.db, app, &projection, seed)?;
    let previous = current(state, app)?;
    if matches!(previous.as_deref(), Some("releasing")) {
        return Err(TakeoverError::State);
    }
    if previous.as_deref() == Some("released") {
        finish_release(state, store, app)?;
    }
    if previous.is_none() || previous.as_deref() == Some("released") {
        let snap = snapshot(app)?;
        if !confirmed && conflict(app, &snap) {
            return Err(TakeoverError::CcSwitchConflict);
        }
        let serialized = serde_json::to_string(&snap).map_err(|_| TakeoverError::Backup)?;
        store
            .set(&account(app), &serialized)
            .map_err(|_| TakeoverError::Backup)?;
        set_state(state, app, "taking_over")?;
    } else if store
        .get(&account(app))
        .map_err(|_| TakeoverError::Backup)?
        .is_none()
    {
        return Err(TakeoverError::Backup);
    }
    controller::enter_with_route(state, app, false, Some("aicli-main"))
        .await
        .map_err(|error| {
            #[cfg(test)]
            eprintln!(
                "AICLI client takeover failed: {}",
                crate::error_for_log(&error)
            );
            log::error!(
                "AICLI client takeover failed: {}",
                crate::error_for_log(&error)
            );
            TakeoverError::Client
        })?;
    set_state(state, app, "active")
}

pub async fn release(
    state: &AppState,
    store: &dyn SecretStore,
    app: &AppType,
) -> Result<(), TakeoverError> {
    let _guard = lock(app)?.lock().await;
    let previous = current(state, app)?;
    if previous.is_none() {
        return Ok(());
    }
    if previous.as_deref() == Some("released") {
        return finish_release(state, store, app);
    }
    let serialized = store
        .get(&account(app))
        .map_err(|_| TakeoverError::Backup)?
        .ok_or(TakeoverError::Backup)?;
    let snap: Snapshot = serde_json::from_str(&serialized).map_err(|_| TakeoverError::Backup)?;
    let paths = files(app)?;
    if snap.version != 1 || snap.files.len() != paths.len() {
        return Err(TakeoverError::Backup);
    }
    set_state(state, app, "releasing")?;
    controller::exit(state, app)
        .await
        .map_err(|_| TakeoverError::Client)?;
    {
        let write = AppWrite::begin(&state.db, app.as_str()).map_err(|_| TakeoverError::Client)?;
        let patches = paths
            .iter()
            .zip(&snap.files)
            .map(|(p, bytes)| {
                Ok(Guarded {
                    expected_pre: digest(
                        read_current(p)
                            .map_err(|_| TakeoverError::Client)?
                            .as_deref(),
                    ),
                    then: bytes.clone().map_or(WholeFile::Delete, WholeFile::Write),
                })
            })
            .collect::<Result<Vec<_>, TakeoverError>>()?;
        let changes = paths
            .iter()
            .zip(&patches)
            .map(|(p, patch)| FileChange {
                file: LiveFile::private(p),
                patch,
            })
            .collect::<Vec<_>>();
        write
            .run("aicli-release", &changes, PendingTarget::default())
            .map_err(|_| TakeoverError::Client)?;
        for (p, expected) in paths.iter().zip(&snap.files) {
            if read_current(p).map_err(|_| TakeoverError::Client)? != *expected {
                return Err(TakeoverError::Verification);
            }
        }
    }
    finish_release(state, store, app)
}

/// Existing controller startup completes live-state pending before resuming our intent.
pub async fn startup(state: &AppState, store: &dyn SecretStore) -> Result<(), TakeoverError> {
    controller::startup(state).await;
    for row in status(state)? {
        let app = row
            .app
            .parse::<AppType>()
            .map_err(|_| TakeoverError::State)?;
        match row.state.as_str() {
            "taking_over" => take_over(state, store, &app, true).await?,
            "releasing" | "released" => release(state, store, &app).await?,
            "active" => (),
            _ => return Err(TakeoverError::State),
        }
    }
    Ok(())
}

/// Keyless seed rows are enriched with public model facts and local credentials only.
pub(crate) fn prepare_projection(
    db: &crate::Database,
    app: &AppType,
    snapshot: &super::runtime::RoutesSnapshot,
    mut provider: crate::provider::Provider,
) -> Result<crate::provider::Provider, TakeoverError> {
    let mut selected = None;
    {
        let conn = db.conn.lock().map_err(|_| TakeoverError::State)?;
        for (scope, key) in [("app", app.as_str()), ("global", "")] {
            selected = conn
                .query_row(
                    "SELECT profile_id FROM aicli_scope_binding WHERE scope=?1 AND scope_key=?2",
                    [scope, key],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .map_err(|_| TakeoverError::State)?;
            if selected.is_some() {
                break;
            }
        }
    }
    let selected = selected
        .or_else(|| snapshot.active_profile_id.clone())
        .unwrap_or_else(|| "recommended".into());
    let entry = snapshot
        .profiles
        .get(&selected)
        .ok_or(TakeoverError::NoModels)?;
    let catalog = snapshot.catalog.as_ref().ok_or(TakeoverError::NoModels)?;
    let protocol = match app {
        AppType::Claude | AppType::ClaudeDesktop => "anthropic",
        AppType::Gemini => "gemini",
        _ => "openai-response",
    };
    let mut models = entry
        .compiled
        .routes
        .iter()
        .filter(|route| {
            !entry
                .compiled
                .repair
                .affected_models
                .contains(&route.public_model)
                && catalog
                    .models
                    .get(&route.upstream_model)
                    .is_some_and(|model| {
                        model.wire.contains_key(protocol)
                            && model.compat.get(protocol)
                                != Some(&super::catalog::Compatibility::Unavailable)
                    })
        })
        .map(|r| r.public_model.clone())
        .collect::<Vec<_>>();
    models.sort();
    models.dedup();
    if models.is_empty() {
        return Err(TakeoverError::NoModels);
    }
    // Claude/Codex/Gemini retain the existing projection path.
    if !matches!(app, AppType::GrokBuild | AppType::ClaudeDesktop) {
        return Ok(provider);
    }
    if *app == AppType::ClaudeDesktop {
        if models.len() > 200 {
            return Err(TakeoverError::TooManyModels);
        }
        for (key, alias) in super::alias::default_aliases(app, &models, Some(db))
            .map_err(|_| TakeoverError::State)?
        {
            provider.settings_config["env"][key] = serde_json::Value::String(alias);
        }
        let mut meta = provider.meta.unwrap_or_default();
        meta.claude_desktop_model_routes.clear();
        for model in models {
            let alias =
                super::alias::encode(app, &model, Some(db)).map_err(|_| TakeoverError::State)?;
            meta.claude_desktop_model_routes.insert(
                alias,
                crate::provider::ClaudeDesktopModelRoute {
                    model: model.clone(),
                    label_override: Some(model),
                    supports_1m: Some(false),
                },
            );
        }
        provider.meta = Some(meta);
    } else {
        let original = read_current(&crate::grok_config::get_grok_config_path())
            .map_err(|_| TakeoverError::Client)?;
        let old_model = original
            .as_deref()
            .and_then(|b| std::str::from_utf8(b).ok())
            .and_then(|text| text.parse::<toml::Value>().ok())
            .and_then(|doc| {
                let name = doc.get("models")?.get("default")?.as_str()?;
                doc.get("model")?
                    .get(name)?
                    .get("model")?
                    .as_str()
                    .map(str::to_owned)
            });
        let chosen =
            choose_grok_model(&models, old_model.as_deref()).ok_or(TakeoverError::NoModels)?;
        let token =
            super::local_auth::token_for(db, app.as_str()).map_err(|_| TakeoverError::State)?;
        let mut doc = toml::map::Map::new();
        doc.insert(
            "models".into(),
            toml::Value::Table(toml::map::Map::from_iter([(
                "default".into(),
                toml::Value::String("aicli".into()),
            )])),
        );
        let table = toml::map::Map::from_iter([
            ("model".into(), toml::Value::String(chosen.into())),
            ("name".into(), toml::Value::String("AICLI".into())),
            (
                "api_backend".into(),
                toml::Value::String(crate::live::project::grok::PROXY_API_BACKEND.into()),
            ),
            ("context_window".into(), toml::Value::Integer(128000)),
            ("api_key".into(), toml::Value::String(token)),
            (
                "base_url".into(),
                toml::Value::String(super::lines::LINES[0].base_url.into()),
            ),
        ]);
        doc.insert(
            "model".into(),
            toml::Value::Table(toml::map::Map::from_iter([(
                "aicli".into(),
                toml::Value::Table(table),
            )])),
        );
        provider.settings_config["config"] =
            serde_json::Value::String(toml::to_string(&doc).map_err(|_| TakeoverError::State)?);
    }
    Ok(provider)
}

fn choose_grok_model<'a>(models: &'a [String], previous: Option<&str>) -> Option<&'a str> {
    previous
        .and_then(|old| models.iter().find(|m| m.as_str() == old))
        .or_else(|| models.iter().find(|m| m.to_lowercase().contains("grok")))
        .or_else(|| models.first())
        .map(String::as_str)
}

pub(crate) fn is_managed(db: &crate::Database, app: &AppType) -> bool {
    db.conn
        .lock()
        .ok()
        .and_then(|conn| {
            conn.query_row(
                "SELECT state FROM aicli_takeover WHERE app_type=?1",
                [app.as_str()],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .ok()
            .flatten()
        })
        .is_some_and(|state| state == "taking_over" || state == "active")
}

pub(crate) fn watch_projection_changes(
    service: crate::services::ProxyService,
    db: std::sync::Arc<crate::Database>,
    mut changes: tokio::sync::watch::Receiver<String>,
) {
    let mut previous = changes.borrow().clone();
    tokio::spawn(async move {
        let mut state = AppState::new(db);
        state.proxy_service = service;
        while changes.changed().await.is_ok() {
            let next = changes.borrow_and_update().clone();
            if next == previous {
                continue;
            }
            previous = next;
            for app in [AppType::GrokBuild, AppType::ClaudeDesktop] {
                if is_managed(&state.db, &app)
                    && controller::resync_route(&state, &app).await.is_err()
                {
                    log::warn!("AICLI client projection refresh failed; retaining existing backup");
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::super::{secrets::MemoryStore, seed};
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    struct Home {
        _dir: tempfile::TempDir,
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }
    impl Home {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let saved = [
                "HOME",
                "USERPROFILE",
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
            Self { _dir: dir, saved }
        }
    }
    impl Drop for Home {
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
    async fn fixture() -> AppState {
        let db = Arc::new(crate::Database::memory().unwrap());
        seed::init(&db).unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute("INSERT INTO aicli_account(id,user_id,username,user_group) VALUES(1,123,'takeover','fixture')",[]).unwrap();
            let data = ["grok-z", "alpha", "sonnet-model"]
                .into_iter()
                .map(|model| {
                    json!({
                        "model_name": model,
                        "enable_groups": ["fixture"],
                        "supported_endpoint_types": ["anthropic", "openai-response", "gemini"]
                    })
                })
                .collect::<Vec<_>>();
            let pricing = json!({"auto_groups":["fixture"],"data":data});
            for (kind, value) in [
                ("groups", json!({"data":{"fixture":{"ratio":1}}})),
                ("pricing", pricing),
            ] {
                conn.execute(
                    "INSERT INTO aicli_cache(kind,fetched_at,body_json) VALUES(?1,1,?2)",
                    rusqlite::params![kind, value.to_string()],
                )
                .unwrap();
            }
        }
        let mut config = db.get_proxy_config().await.unwrap();
        config.listen_port = 0;
        db.update_proxy_config(config).await.unwrap();
        AppState::new(db)
    }
    fn seed_files(app: &AppType) {
        for p in files(app)
            .unwrap()
            .into_iter()
            .take(if *app == AppType::Codex {
                3
            } else {
                usize::MAX
            })
        {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            let text = if p.extension().is_some_and(|e| e == "toml") {
                "# Original formatting\nuser_value = true\n"
            } else if p.file_name().is_some_and(|n| n == ".env") {
                "# Original env\nUSER_VALUE=keep\n"
            } else if p.file_name().is_some_and(|n| n == "_meta.json") {
                "{\"entries\":[],\"note\": \"keep\"}\n"
            } else {
                "{\"note\": \"keep\"}\n"
            };
            std::fs::write(p, text).unwrap();
        }
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn five_clients_restore_exact_bytes_and_keep_seed_rows_keyless() {
        for app in controller::PROXY_APPS {
            let _home = Home::new();
            let state = fixture().await;
            let store = MemoryStore::default();
            seed_files(&app);
            let before = snapshot(&app).unwrap();
            let seed_before = state
                .db
                .get_provider_by_id("aicli-main", app.as_str())
                .unwrap()
                .unwrap()
                .settings_config;
            take_over(&state, &store, &app, false).await.unwrap();
            let after = snapshot(&app).unwrap();
            let text = after
                .files
                .iter()
                .flatten()
                .map(|b| String::from_utf8_lossy(b))
                .collect::<Vec<_>>()
                .join("\n");
            assert!(text.contains("127.0.0.1:15821"), "{}", app.as_str());
            let token = super::super::local_auth::token_for(&state.db, app.as_str()).unwrap();
            assert!(text.contains(&token));
            assert!(!text.contains("sk-"));
            assert_eq!(
                state
                    .db
                    .get_provider_by_id("aicli-main", app.as_str())
                    .unwrap()
                    .unwrap()
                    .settings_config,
                seed_before
            );
            assert!(store.get(&account(&app)).unwrap().is_some());
            release(&state, &store, &app).await.unwrap();
            assert_eq!(
                snapshot(&app).unwrap().files,
                before.files,
                "{}",
                app.as_str()
            );
            assert!(store.get(&account(&app)).unwrap().is_none());
            assert!(status(&state).unwrap().is_empty());
        }
    }
    #[test]
    fn grok_default_selection_three_levels() {
        let models = vec!["alpha".into(), "GROK-a".into(), "grok-z".into()];
        assert_eq!(choose_grok_model(&models, Some("grok-z")), Some("grok-z"));
        assert_eq!(choose_grok_model(&models, Some("absent")), Some("GROK-a"));
        assert_eq!(choose_grok_model(&models[..1], None), Some("alpha"));
        assert_eq!(choose_grok_model(&[], None), None);
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn projection_models_and_empty_profile_are_exact() {
        let _home = Home::new();
        let state = fixture().await;
        let runtime = super::super::runtime::AicliRuntime::new(state.db.clone());
        let snap = runtime.cached_projection().unwrap();
        let app = AppType::ClaudeDesktop;
        let seed = state
            .db
            .get_provider_by_id("aicli-main", app.as_str())
            .unwrap()
            .unwrap();
        let row = prepare_projection(&state.db, &app, &snap, seed.clone()).unwrap();
        let profile = crate::claude_desktop_config::route_profile(
            &state.db,
            &row,
            "http://127.0.0.1:15821",
            &[],
        )
        .unwrap();
        let mut actual = profile["inferenceModels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        let mut expected = ["alpha", "grok-z", "sonnet-model"]
            .into_iter()
            .map(|m| super::super::alias::encode(&app, m, Some(&state.db)).unwrap())
            .collect::<Vec<_>>();
        actual.sort();
        expected.sort();
        assert_eq!(actual, expected);
        assert!(!profile.to_string().contains("sk-"));
        let app = AppType::GrokBuild;
        let seed = state
            .db
            .get_provider_by_id("aicli-main", app.as_str())
            .unwrap()
            .unwrap();
        let row = prepare_projection(&state.db, &app, &snap, seed).unwrap();
        let doc = row.settings_config["config"]
            .as_str()
            .unwrap()
            .parse::<toml::Value>()
            .unwrap();
        assert_eq!(
            doc["model"]["aicli"]["context_window"].as_integer(),
            Some(128000)
        );
        assert_eq!(doc["model"]["aicli"]["model"].as_str(), Some("grok-z"));
        state
            .db
            .conn
            .lock()
            .unwrap()
            .execute("DELETE FROM aicli_cache WHERE kind='pricing'", [])
            .unwrap();
        seed_files(&app);
        let before = snapshot(&app).unwrap();
        assert_eq!(
            take_over(&state, &MemoryStore::default(), &app, false)
                .await
                .unwrap_err(),
            TakeoverError::NoModels
        );
        assert_eq!(snapshot(&app).unwrap().files, before.files);
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn legacy_conflict_requires_confirmation_and_preserves_original_backup() {
        let _home = Home::new();
        let state = fixture().await;
        let store = MemoryStore::default();
        let app = AppType::Claude;
        let p = files(&app).unwrap().remove(0);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, br#"{"env":{"ANTHROPIC_AUTH_TOKEN":"PROXY_MANAGED"}}"#).unwrap();
        let before = snapshot(&app).unwrap();
        assert_eq!(
            take_over(&state, &store, &app, false).await.unwrap_err(),
            TakeoverError::CcSwitchConflict
        );
        assert_eq!(snapshot(&app).unwrap().files, before.files);
        assert!(status(&state).unwrap().is_empty());
        take_over(&state, &store, &app, true).await.unwrap();
        take_over(&state, &store, &app, true).await.unwrap();
        release(&state, &store, &app).await.unwrap();
        assert_eq!(snapshot(&app).unwrap().files, before.files);
        let desktop = Snapshot {
            version: 1,
            files: vec![Some(
                format!("{{\"appliedId\":\"{LEGACY_CC_SWITCH_DESKTOP_PROFILE_ID}\"}}").into_bytes(),
            )],
        };
        assert!(conflict(&AppType::ClaudeDesktop, &desktop));
        let url = Snapshot {
            version: 1,
            files: vec![Some(
                format!("http://127.0.0.1:{LEGACY_CC_SWITCH_PORT}/v1").into_bytes(),
            )],
        };
        assert!(conflict(&AppType::Codex, &url));
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn pending_takeover_recovers_on_startup_then_restores_original_bytes() {
        let _home = Home::new();
        let state = fixture().await;
        let store = MemoryStore::default();
        let app = AppType::Gemini;
        seed_files(&app);
        let before = snapshot(&app).unwrap();
        crate::mode::operation::failpoint::crash_at(Some("published:0"));
        let result = take_over(&state, &store, &app, false).await;
        crate::mode::operation::failpoint::crash_at(None);
        assert!(result.is_err());
        assert_eq!(
            current(&state, &app).unwrap().as_deref(),
            Some("taking_over")
        );
        startup(&state, &store).await.unwrap();
        assert_eq!(current(&state, &app).unwrap().as_deref(), Some("active"));
        assert!(crate::mode::state::apps_with_pending(
            &crate::live::engine::DeviceStore::for_device()
        )
        .unwrap()
        .is_empty());
        release(&state, &store, &app).await.unwrap();
        assert_eq!(snapshot(&app).unwrap().files, before.files);
    }
    #[tokio::test]
    #[serial_test::serial]
    async fn fingerprint_change_resyncs_desktop_and_grok_without_replacing_backup() {
        let _home = Home::new();
        let state = fixture().await;
        let store = MemoryStore::default();
        for app in [AppType::GrokBuild, AppType::ClaudeDesktop] {
            seed_files(&app);
            take_over(&state, &store, &app, false).await.unwrap();
        }
        let backups = [AppType::GrokBuild, AppType::ClaudeDesktop]
            .map(|app| store.get(&account(&app)).unwrap());
        let runtime = super::super::runtime::AicliRuntime::new(state.db.clone());
        let old = runtime.cached_projection().unwrap().fingerprint.clone();
        let (changes, receiver) = tokio::sync::watch::channel(old.clone());
        watch_projection_changes(state.proxy_service.clone(), state.db.clone(), receiver);
        // Ensure the subscriber has captured the old fingerprint before publication.
        tokio::task::yield_now().await;
        let pricing = json!({"auto_groups":["fixture"],"data":[{"model_name":"replacement-grok","enable_groups":["fixture"],"supported_endpoint_types":["anthropic","openai-response"]}]});
        state
            .db
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE aicli_cache SET body_json=?1 WHERE kind='pricing'",
                [pricing.to_string()],
            )
            .unwrap();
        let next = runtime.cached_projection().unwrap().fingerprint.clone();
        assert_ne!(old, next);
        changes.send_replace(next);
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let grok =
                    std::fs::read_to_string(crate::grok_config::get_grok_config_path()).unwrap();
                let desktop = snapshot(&AppType::ClaudeDesktop)
                    .unwrap()
                    .files
                    .into_iter()
                    .flatten()
                    .any(|b| String::from_utf8_lossy(&b).contains("replacement-grok"));
                if grok.contains("replacement-grok") && desktop {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("fingerprint publication must refresh both managed clients");
        for (app, backup) in [AppType::GrokBuild, AppType::ClaudeDesktop]
            .into_iter()
            .zip(backups)
        {
            assert_eq!(store.get(&account(&app)).unwrap(), backup);
            release(&state, &store, &app).await.unwrap();
        }
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn desktop_over_limit_and_empty_catalog_never_touch_client_files() {
        let _home = Home::new();
        let state = fixture().await;
        let runtime = super::super::runtime::AicliRuntime::new(state.db.clone());
        let pricing = json!({"auto_groups":["fixture"],"data":(0..201).map(|n|json!({"model_name":format!("model-{n:03}"),"enable_groups":["fixture"],"supported_endpoint_types":["anthropic"]})).collect::<Vec<_>>()});
        state
            .db
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE aicli_cache SET body_json=?1 WHERE kind='pricing'",
                [pricing.to_string()],
            )
            .unwrap();
        let app = AppType::ClaudeDesktop;
        seed_files(&app);
        let before = snapshot(&app).unwrap();
        let seed = state
            .db
            .get_provider_by_id("aicli-main", app.as_str())
            .unwrap()
            .unwrap();
        assert_eq!(
            prepare_projection(&state.db, &app, &runtime.cached_projection().unwrap(), seed)
                .unwrap_err(),
            TakeoverError::TooManyModels
        );
        assert_eq!(snapshot(&app).unwrap().files, before.files);
        state
            .db
            .conn
            .lock()
            .unwrap()
            .execute("DELETE FROM aicli_cache WHERE kind='pricing'", [])
            .unwrap();
        for app in controller::PROXY_APPS {
            seed_files(&app);
            let before = snapshot(&app).unwrap();
            let store = MemoryStore::default();
            assert_eq!(
                take_over(&state, &store, &app, false).await.unwrap_err(),
                TakeoverError::NoModels
            );
            assert_eq!(snapshot(&app).unwrap().files, before.files);
            assert!(store.get(&account(&app)).unwrap().is_none());
        }
    }
}
