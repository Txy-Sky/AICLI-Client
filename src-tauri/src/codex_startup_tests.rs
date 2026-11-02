use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

struct IsolatedHome {
    saved_env: Vec<(&'static str, Option<OsString>)>,
    dir: tempfile::TempDir,
}

impl IsolatedHome {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("create isolated startup HOME");
        let keys = [
            "HOME",
            "USERPROFILE",
            "CC_SWITCH_TEST_HOME",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "CODEX_HOME",
            "CODEX_SQLITE_HOME",
        ];
        let saved_env = keys
            .into_iter()
            .map(|key| (key, std::env::var_os(key)))
            .collect();
        let isolated = Self { saved_env, dir };
        for key in ["HOME", "USERPROFILE", "CC_SWITCH_TEST_HOME"] {
            std::env::set_var(key, isolated.dir.path());
        }
        for (key, path) in [
            ("XDG_CONFIG_HOME", ".config"),
            ("XDG_DATA_HOME", ".local/share"),
            ("XDG_CACHE_HOME", ".cache"),
            ("CODEX_HOME", ".codex"),
            ("CODEX_SQLITE_HOME", ".codex"),
        ] {
            std::env::set_var(key, isolated.dir.path().join(path));
        }
        crate::settings::reload_settings().expect("load fresh isolated settings");
        isolated
    }
}

impl Drop for IsolatedHome {
    fn drop(&mut self) {
        for (key, value) in &self.saved_env {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
        let _ = crate::settings::reload_settings();
    }
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).expect("read isolated directory") {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                visit(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root).expect("relative path").to_path_buf(),
                    fs::read(path).expect("read original bytes"),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

#[test]
#[serial_test::serial]
fn codex_startup_migrations_preserve_existing_history_bytes() {
    let home = IsolatedHome::new();
    let codex_dir = home.dir.path().join(".codex");
    let sessions = codex_dir.join("sessions/2026/11/02");
    fs::create_dir_all(&sessions).expect("create sessions");
    assert_eq!(crate::codex_config::get_codex_config_dir(), codex_dir);
    assert!(!crate::settings::unify_codex_session_history());
    fs::write(
        codex_dir.join("config.toml"),
        "model_provider = \"ccswitch\"\r\n# preserve exact bytes\r\n",
    )
    .expect("write original config");
    let providers = [
        "ccswitch",
        "openrouter",
        "cc-switch-official",
        "aicli-official",
    ];
    {
        let state_path = codex_dir.join(crate::codex_state_db::CODEX_STATE_DB_FILENAME);
        let conn = rusqlite::Connection::open(state_path).expect("create local Codex state DB");
        conn.execute_batch(
            "CREATE TABLE threads (id TEXT PRIMARY KEY, model_provider TEXT NOT NULL);",
        )
        .expect("create threads");
        for provider in providers {
            let id = format!("session-{provider}");
            let meta = serde_json::json!({
                "type": "session_meta",
                "payload": {"id": id, "model_provider": provider}
            });
            fs::write(
                sessions.join(format!("{id}.jsonl")),
                format!("{meta}\r\n{{\"type\":\"response_item\",\"payload\":{{\"text\":\"unchanged\"}}}}\r\n"),
            )
            .expect("write original session");
            conn.execute(
                "INSERT INTO threads (id, model_provider) VALUES (?1, ?2)",
                [id.as_str(), provider],
            )
            .expect("seed local Codex thread");
        }
    }
    let db = crate::Database::memory().expect("fresh AICLI database");
    assert!(db
        .get_all_providers("codex")
        .expect("fresh providers")
        .is_empty());
    let before = snapshot(home.dir.path());
    assert_eq!(before.len(), providers.len() + 2);

    super::run_codex_startup_migrations(&db);

    assert_eq!(
        snapshot(home.dir.path()),
        before,
        "all startup files unchanged"
    );
    assert!(db
        .get_all_providers("codex")
        .expect("providers after startup")
        .is_empty());
}
