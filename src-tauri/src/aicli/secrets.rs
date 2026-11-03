//! Account credentials stay outside SQLite. Error and debug text never contain values.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

pub const KEYCHAIN_SERVICE: &str = "com.aicliapi.client";

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SecretError {
    #[error("Credential storage is unavailable")]
    Unavailable,
    #[error("Credential file could not be read")]
    Read,
    #[error("Credential file could not be written")]
    Write,
    #[error("Credential file is invalid")]
    InvalidFile,
}

pub trait SecretStore: fmt::Debug + Send + Sync {
    fn get(&self, account: &str) -> Result<Option<String>, SecretError>;
    fn set(&self, account: &str, value: &str) -> Result<(), SecretError>;
    fn delete(&self, account: &str) -> Result<(), SecretError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretBackend {
    Keychain,
    FileFallback,
}

#[derive(Debug)]
struct NativeKeyring;

impl NativeKeyring {
    fn entry(account: &str) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(KEYCHAIN_SERVICE, account).map_err(|_| SecretError::Unavailable)
    }
}

impl SecretStore for NativeKeyring {
    fn get(&self, account: &str) -> Result<Option<String>, SecretError> {
        match Self::entry(account)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(SecretError::Unavailable),
        }
    }

    fn set(&self, account: &str, value: &str) -> Result<(), SecretError> {
        Self::entry(account)?
            .set_password(value)
            .map_err(|_| SecretError::Unavailable)
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        match Self::entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(SecretError::Unavailable),
        }
    }
}

/// Serializes all file read/modify/write operations, including separate store instances.
static FILE_LOCK: Mutex<()> = Mutex::new(());

pub struct FileFallbackStore {
    path: PathBuf,
}

impl fmt::Debug for FileFallbackStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FileFallbackStore { .. }")
    }
}

impl Default for FileFallbackStore {
    fn default() -> Self {
        Self {
            path: crate::config::get_home_dir().join(".aicli-client/secrets.json"),
        }
    }
}

impl FileFallbackStore {
    fn read(&self) -> Result<BTreeMap<String, String>, SecretError> {
        match fs::symlink_metadata(&self.path) {
            Ok(metadata) if !metadata.file_type().is_file() => {
                return Err(SecretError::InvalidFile)
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(BTreeMap::new())
            }
            Err(_) => return Err(SecretError::Read),
        }
        let bytes = fs::read(&self.path).map_err(|_| SecretError::Read)?;
        serde_json::from_slice(&bytes).map_err(|_| SecretError::InvalidFile)
    }

    fn write(&self, values: &BTreeMap<String, String>) -> Result<(), SecretError> {
        let parent = self.path.parent().ok_or(SecretError::Write)?;
        fs::create_dir_all(parent).map_err(|_| SecretError::Write)?;
        // NamedTempFile is private on Unix, so credentials are never briefly world-readable.
        let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(|_| SecretError::Write)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            staged
                .as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|_| SecretError::Write)?;
        }
        serde_json::to_writer(staged.as_file_mut(), values).map_err(|_| SecretError::Write)?;
        staged.flush().map_err(|_| SecretError::Write)?;
        staged
            .as_file()
            .sync_all()
            .map_err(|_| SecretError::Write)?;
        staged.persist(&self.path).map_err(|_| SecretError::Write)?;
        Ok(())
    }
}

impl SecretStore for FileFallbackStore {
    fn get(&self, account: &str) -> Result<Option<String>, SecretError> {
        let _guard = FILE_LOCK.lock().map_err(|_| SecretError::Unavailable)?;
        Ok(self.read()?.remove(account))
    }

    fn set(&self, account: &str, value: &str) -> Result<(), SecretError> {
        let _guard = FILE_LOCK.lock().map_err(|_| SecretError::Unavailable)?;
        let mut values = self.read()?;
        values.insert(account.to_owned(), value.to_owned());
        self.write(&values)
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        let _guard = FILE_LOCK.lock().map_err(|_| SecretError::Unavailable)?;
        let mut values = self.read()?;
        if values.remove(account).is_some() {
            self.write(&values)?;
        }
        Ok(())
    }
}

/// A failure permanently selects file storage for this instance; missing keys are not failures.
pub struct KeyringStore {
    primary: Box<dyn SecretStore>,
    fallback: FileFallbackStore,
    backend: Mutex<SecretBackend>,
}

impl fmt::Debug for KeyringStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyringStore")
            .field("backend", &self.secret_backend())
            .finish_non_exhaustive()
    }
}

impl Default for KeyringStore {
    fn default() -> Self {
        Self::with_primary(Box::new(NativeKeyring), FileFallbackStore::default())
    }
}

impl KeyringStore {
    fn with_primary(primary: Box<dyn SecretStore>, fallback: FileFallbackStore) -> Self {
        // An existing fallback file is the durable record of a previous downgrade.
        // Do not silently lose those credentials when a formerly unavailable keychain returns.
        let backend = match fallback.path.symlink_metadata() {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => SecretBackend::Keychain,
            _ => SecretBackend::FileFallback,
        };
        Self {
            primary,
            fallback,
            backend: Mutex::new(backend),
        }
    }
    pub fn secret_backend(&self) -> SecretBackend {
        *self
            .backend
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn access<T>(
        &self,
        operation: impl Fn(&dyn SecretStore) -> Result<T, SecretError>,
    ) -> Result<T, SecretError> {
        let mut backend = self.backend.lock().map_err(|_| SecretError::Unavailable)?;
        if *backend == SecretBackend::Keychain {
            if let Ok(value) = operation(self.primary.as_ref()) {
                return Ok(value);
            }
            *backend = SecretBackend::FileFallback;
        }
        operation(&self.fallback)
    }
}

impl SecretStore for KeyringStore {
    fn get(&self, account: &str) -> Result<Option<String>, SecretError> {
        self.access(|store| store.get(account))
    }

    fn set(&self, account: &str, value: &str) -> Result<(), SecretError> {
        self.access(|store| store.set(account, value))
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        self.access(|store| store.delete(account))
    }
}

#[cfg(test)]
#[derive(Default)]
pub struct MemoryStore(Mutex<BTreeMap<String, String>>);

#[cfg(test)]
impl fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MemoryStore { .. }")
    }
}

#[cfg(test)]
impl SecretStore for MemoryStore {
    fn get(&self, account: &str) -> Result<Option<String>, SecretError> {
        Ok(self.0.lock().unwrap().get(account).cloned())
    }
    fn set(&self, account: &str, value: &str) -> Result<(), SecretError> {
        self.0
            .lock()
            .unwrap()
            .insert(account.to_owned(), value.to_owned());
        Ok(())
    }
    fn delete(&self, account: &str) -> Result<(), SecretError> {
        self.0.lock().unwrap().remove(account);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    // Synthetic test data only; never uses environment credentials or the host keychain.
    const VALUE: &str = "synthetic-credential-DO-NOT-PRINT";

    fn file_store(temp: &tempfile::TempDir) -> FileFallbackStore {
        FileFallbackStore {
            path: temp.path().join(".aicli-client/secrets.json"),
        }
    }

    fn round_trip(store: &dyn SecretStore) {
        assert_eq!(store.get("access-token").unwrap(), None);
        store.delete("access-token").unwrap();
        store.set("access-token", VALUE).unwrap();
        store.set("token:7", "synthetic-other").unwrap();
        assert_eq!(store.get("access-token").unwrap().as_deref(), Some(VALUE));
        store.set("access-token", "synthetic-replacement").unwrap();
        assert_eq!(
            store.get("access-token").unwrap().as_deref(),
            Some("synthetic-replacement")
        );
        store.delete("access-token").unwrap();
        assert_eq!(store.get("access-token").unwrap(), None);
        assert_eq!(
            store.get("token:7").unwrap().as_deref(),
            Some("synthetic-other")
        );
    }

    #[test]
    fn memory_store_crud_and_debug_redaction() {
        let store = MemoryStore::default();
        round_trip(&store);
        store.set("access-token", VALUE).unwrap();
        assert!(!format!("{store:?}").contains(VALUE));
    }

    #[test]
    fn fallback_file_crud_survives_reopening_and_redacts_debug() {
        let temp = tempfile::tempdir().unwrap();
        let store = file_store(&temp);
        round_trip(&store);
        store.set("access-token", VALUE).unwrap();
        assert_eq!(
            file_store(&temp).get("access-token").unwrap().as_deref(),
            Some(VALUE)
        );
        assert!(!format!("{store:?}").contains(VALUE));
    }

    #[cfg(unix)]
    #[test]
    fn fallback_file_permissions_remain_private_on_replacement() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let store = file_store(&temp);
        store.set("access-token", VALUE).unwrap();
        assert_eq!(
            fs::metadata(&store.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::set_permissions(&store.path, fs::Permissions::from_mode(0o644)).unwrap();
        store.set("token:7", "synthetic-other").unwrap();
        assert_eq!(
            fs::metadata(&store.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[derive(Debug)]
    struct Unavailable(Arc<AtomicUsize>);

    impl SecretStore for Unavailable {
        fn get(&self, _: &str) -> Result<Option<String>, SecretError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(SecretError::Unavailable)
        }
        fn set(&self, _: &str, _: &str) -> Result<(), SecretError> {
            self.get("").map(|_| ())
        }
        fn delete(&self, _: &str) -> Result<(), SecretError> {
            self.get("").map(|_| ())
        }
    }

    #[test]
    fn each_failed_operation_selects_fallback_once() {
        for first in ["get", "set", "delete"] {
            let temp = tempfile::tempdir().unwrap();
            let calls = Arc::new(AtomicUsize::new(0));
            let store = KeyringStore {
                primary: Box::new(Unavailable(calls.clone())),
                fallback: file_store(&temp),
                backend: Mutex::new(SecretBackend::Keychain),
            };
            assert_eq!(store.secret_backend(), SecretBackend::Keychain);
            match first {
                "get" => {
                    store.get("access-token").unwrap();
                }
                "set" => store.set("access-token", VALUE).unwrap(),
                _ => store.delete("access-token").unwrap(),
            }
            assert_eq!(store.secret_backend(), SecretBackend::FileFallback);
            store.delete("access-token").unwrap();
            round_trip(&store);
            store.set("access-token", VALUE).unwrap();
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert!(!format!("{store:?}").contains(VALUE));
        }
    }

    #[test]
    fn missing_key_does_not_trigger_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let store = KeyringStore {
            primary: Box::<MemoryStore>::default(),
            fallback: file_store(&temp),
            backend: Mutex::new(SecretBackend::Keychain),
        };
        round_trip(&store);
        assert_eq!(store.secret_backend(), SecretBackend::Keychain);
        assert!(!store.fallback.path.exists());
        assert_eq!(
            serde_json::to_string(&store.secret_backend()).unwrap(),
            "\"keychain\""
        );
        assert_eq!(
            serde_json::to_string(&SecretBackend::FileFallback).unwrap(),
            "\"file_fallback\""
        );
    }

    #[test]
    fn reopening_fallback_does_not_lose_credentials_when_keychain_recovers_empty() {
        let temp = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let store =
            KeyringStore::with_primary(Box::new(Unavailable(calls.clone())), file_store(&temp));
        store.set("access-token", VALUE).unwrap();
        drop(store);
        let reopened = KeyringStore::with_primary(Box::<MemoryStore>::default(), file_store(&temp));
        assert_eq!(reopened.secret_backend(), SecretBackend::FileFallback);
        assert_eq!(
            reopened.get("access-token").unwrap().as_deref(),
            Some(VALUE)
        );
        reopened.delete("access-token").unwrap();
        assert_eq!(reopened.get("access-token").unwrap(), None);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn invalid_file_errors_are_redacted_and_do_not_destroy_data() {
        let temp = tempfile::tempdir().unwrap();
        let store = file_store(&temp);
        fs::create_dir_all(store.path.parent().unwrap()).unwrap();
        fs::write(&store.path, VALUE).unwrap();
        for error in [
            store.get("access-token").unwrap_err(),
            store.set("access-token", VALUE).unwrap_err(),
            store.delete("access-token").unwrap_err(),
        ] {
            assert_eq!(error, SecretError::InvalidFile);
            assert!(!format!("{error} {error:?}").contains(VALUE));
        }
        assert_eq!(fs::read_to_string(&store.path).unwrap(), VALUE);
    }

    #[test]
    fn concurrent_file_store_instances_preserve_all_accounts() {
        let temp = tempfile::tempdir().unwrap();
        std::thread::scope(|scope| {
            for i in 0..8 {
                let store = file_store(&temp);
                scope.spawn(move || store.set(&format!("token:{i}"), VALUE).unwrap());
            }
        });
        let store = file_store(&temp);
        for i in 0..8 {
            assert_eq!(
                store.get(&format!("token:{i}")).unwrap().as_deref(),
                Some(VALUE)
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn fallback_refuses_symlink_without_reading_or_overwriting_target() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("other-app.json");
        fs::write(&target, VALUE).unwrap();
        let store = file_store(&temp);
        fs::create_dir_all(store.path.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&target, &store.path).unwrap();
        assert_eq!(store.get("access-token"), Err(SecretError::InvalidFile));
        assert_eq!(
            store.set("access-token", VALUE),
            Err(SecretError::InvalidFile)
        );
        assert_eq!(fs::read_to_string(target).unwrap(), VALUE);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn native_secret_service_unavailable_uses_isolated_file() {
        const CHILD: &str = "AICLI_SECRET_SERVICE_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let store = KeyringStore::default();
            round_trip(&store);
            assert_eq!(store.secret_backend(), SecretBackend::FileFallback);
            assert!(FileFallbackStore::default().path.is_file());
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        // A separate process prevents environment races and cannot reach a real session bus.
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "aicli::secrets::tests::native_secret_service_unavailable_uses_isolated_file",
            ])
            .env(CHILD, "1")
            .env("HOME", temp.path())
            .env("CC_SWITCH_TEST_HOME", temp.path())
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={}/absent-bus", temp.path().display()),
            )
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "isolated native backend test failed"
        );
        assert!(temp.path().join(".aicli-client/secrets.json").is_file());
    }
}
