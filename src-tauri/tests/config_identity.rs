use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;

use cc_switch_lib::live::engine::DeviceStore;
use cc_switch_lib::{Database, APP_DIR_NAME};

mod support;
use support::{ensure_test_home, reset_test_fs, test_mutex};

struct HomeOverride(Option<OsString>);

impl Drop for HomeOverride {
    fn drop(&mut self) {
        match self.0.take() {
            Some(previous) => std::env::set_var("HOME", previous),
            None => std::env::remove_var("HOME"),
        }
    }
}

#[test]
fn database_and_device_store_use_only_the_aicli_directory() {
    let _guard = test_mutex().lock().unwrap_or_else(|e| e.into_inner());
    reset_test_fs();
    let home = ensure_test_home();
    assert_eq!(APP_DIR_NAME, ".aicli-client");
    assert_eq!(
        PathBuf::from(std::env::var_os("CC_SWITCH_TEST_HOME").unwrap()),
        home
    );
    assert_eq!(PathBuf::from(std::env::var_os("HOME").unwrap()), home);

    let db = Database::init().expect("initialize isolated AICLI database");
    let app_dir = home.join(".aicli-client");
    let store = DeviceStore::for_device();
    assert!(app_dir.join("aicli-client.db").is_file());
    assert!(app_dir.join("settings.json").is_file());
    assert_eq!(store.state_path(), app_dir.join("live-state.json"));
    assert_eq!(
        store.first_write_backup_dir(),
        app_dir.join("backups/live-first-write")
    );
    assert!(!home.join(".cc-switch").exists());
    assert!(!app_dir.join("cc-switch.db").exists());
    drop(db);
    assert!(!home.join(".cc-switch").exists());
}

#[test]
fn existing_upstream_database_in_another_home_is_never_adopted() {
    let _guard = test_mutex().lock().unwrap_or_else(|e| e.into_inner());
    reset_test_fs();
    let home = ensure_test_home();
    let foreign_home = tempfile::tempdir().expect("create foreign HOME");
    let upstream_dir = foreign_home.path().join(".cc-switch");
    fs::create_dir(&upstream_dir).expect("create upstream fixture directory");
    let upstream_db = upstream_dir.join("cc-switch.db");
    let original = b"upstream database must remain untouched";
    fs::write(&upstream_db, original).expect("seed upstream sentinel");
    let _restore_home = HomeOverride(std::env::var_os("HOME"));
    std::env::set_var("HOME", foreign_home.path());

    let db = Database::init().expect("initialize under explicit test home");
    let store = DeviceStore::for_device();
    assert!(home.join(".aicli-client/aicli-client.db").is_file());
    assert_eq!(
        store.state_path(),
        home.join(".aicli-client/live-state.json")
    );
    assert_eq!(fs::read(&upstream_db).unwrap(), original);
    assert_eq!(fs::read_dir(&upstream_dir).unwrap().count(), 1);
    assert!(!foreign_home.path().join(".aicli-client").exists());
    assert!(!home.join(".cc-switch").exists());
    drop(db);
}
