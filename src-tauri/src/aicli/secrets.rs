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
    /// 当前实际使用的存储后端（F-06 设置页的降级提示）；不区分后端的实现返回 None。
    fn backend(&self) -> Option<SecretBackend> {
        None
    }
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

/// Longest value written to one native credential. Windows Credential Manager rejects blobs over
/// 2560 bytes (stored as UTF-16, so about 1280 characters); takeover backups of client config
/// files are far larger. 1000 characters leaves headroom on every platform.
const NATIVE_CHUNK_CHARS: usize = 1000;
/// Marks a value split across `<account>#part<i>` entries; the suffix is the part count.
const CHUNK_HEADER: &str = "aicli-chunked-v1:";

/// Splits values longer than `max` characters across several entries of `inner`.
///
/// Short values are stored unchanged, so credentials written before this existed still read.
/// Parts are written before the header and removed after it, so a crash leaves either the old
/// value readable or orphan parts that the next write of the same account cleans up.
struct Chunked {
    inner: Box<dyn SecretStore>,
    max: usize,
}

impl fmt::Debug for Chunked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Chunked { .. }")
    }
}

impl Chunked {
    fn part(account: &str, index: usize) -> String {
        format!("{account}#part{index}")
    }

    fn part_count(value: &str) -> Option<usize> {
        value.strip_prefix(CHUNK_HEADER)?.parse().ok()
    }

    fn stored_parts(&self, account: &str) -> Result<usize, SecretError> {
        Ok(self
            .inner
            .get(account)?
            .as_deref()
            .and_then(Self::part_count)
            .unwrap_or(0))
    }

    fn delete_parts(&self, account: &str, from: usize, to: usize) -> Result<(), SecretError> {
        for index in from..to {
            self.inner.delete(&Self::part(account, index))?;
        }
        Ok(())
    }
}

impl SecretStore for Chunked {
    fn get(&self, account: &str) -> Result<Option<String>, SecretError> {
        let Some(value) = self.inner.get(account)? else {
            return Ok(None);
        };
        let Some(count) = Self::part_count(&value) else {
            return Ok(Some(value));
        };
        let mut joined = String::new();
        for index in 0..count {
            joined.push_str(
                &self
                    .inner
                    .get(&Self::part(account, index))?
                    .ok_or(SecretError::Read)?,
            );
        }
        Ok(Some(joined))
    }

    fn set(&self, account: &str, value: &str) -> Result<(), SecretError> {
        let previous = self.stored_parts(account)?;
        let chars: Vec<char> = value.chars().collect();
        if chars.len() <= self.max && Self::part_count(value).is_none() {
            self.inner.set(account, value)?;
            return self.delete_parts(account, 0, previous);
        }
        let parts: Vec<String> = chars
            .chunks(self.max)
            .map(|chunk| chunk.iter().collect())
            .collect();
        for (index, part) in parts.iter().enumerate() {
            self.inner.set(&Self::part(account, index), part)?;
        }
        self.inner
            .set(account, &format!("{CHUNK_HEADER}{}", parts.len()))?;
        self.delete_parts(account, parts.len(), previous)
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        let previous = self.stored_parts(account)?;
        self.inner.delete(account)?;
        self.delete_parts(account, 0, previous)
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
    /// The native store failed in this process. Until then a store that starts in file mode
    /// (because the file exists) still reads keys the file lacks from the native store.
    primary_failed: std::sync::atomic::AtomicBool,
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
        Self::with_primary(
            Box::new(Chunked {
                inner: Box::new(NativeKeyring),
                max: NATIVE_CHUNK_CHARS,
            }),
            FileFallbackStore::default(),
        )
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
            primary_failed: std::sync::atomic::AtomicBool::new(false),
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
            self.mark_primary_failed();
        }
        operation(&self.fallback)
    }

    fn mark_primary_failed(&self) {
        self.primary_failed
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// In file mode, the native store may still hold keys written before the downgrade (for
    /// example the access token stored at bind time, before a large takeover backup failed on
    /// Windows and created the file). Reading through keeps those credentials instead of
    /// reporting them missing.
    fn native_still_usable(&self) -> bool {
        self.secret_backend() == SecretBackend::FileFallback
            && !self
                .primary_failed
                .load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl SecretStore for KeyringStore {
    fn get(&self, account: &str) -> Result<Option<String>, SecretError> {
        let value = self.access(|store| store.get(account))?;
        if value.is_some() || !self.native_still_usable() {
            return Ok(value);
        }
        match self.primary.get(account) {
            Ok(native) => Ok(native),
            Err(_) => {
                self.mark_primary_failed();
                Ok(None)
            }
        }
    }

    fn backend(&self) -> Option<SecretBackend> {
        Some(self.secret_backend())
    }

    fn set(&self, account: &str, value: &str) -> Result<(), SecretError> {
        self.access(|store| store.set(account, value))
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        self.access(|store| store.delete(account))?;
        // A copy left in the native store would be read back through `get`.
        if self.native_still_usable() && self.primary.delete(account).is_err() {
            self.mark_primary_failed();
        }
        Ok(())
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
                primary_failed: Default::default(),
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

    /// A native store that, like Windows Credential Manager, rejects long values. Shared so a
    /// "restarted" KeyringStore sees what the previous one wrote.
    #[derive(Debug, Clone, Default)]
    struct Limited(Arc<MemoryStore>, usize);

    impl SecretStore for Limited {
        fn get(&self, account: &str) -> Result<Option<String>, SecretError> {
            self.0.get(account)
        }
        fn set(&self, account: &str, value: &str) -> Result<(), SecretError> {
            if value.chars().count() > self.1 {
                return Err(SecretError::Unavailable);
            }
            self.0.set(account, value)
        }
        fn delete(&self, account: &str) -> Result<(), SecretError> {
            self.0.delete(account)
        }
    }

    fn limited(max: usize) -> Limited {
        Limited(Arc::default(), max)
    }

    fn chunked(native: &Limited) -> Box<dyn SecretStore> {
        Box::new(Chunked {
            inner: Box::new(native.clone()),
            max: 10,
        })
    }

    fn entries(native: &Limited) -> Vec<String> {
        native.0 .0.lock().unwrap().keys().cloned().collect()
    }

    #[test]
    fn chunked_store_splits_long_values_and_cleans_up_parts() {
        let native = limited(10);
        let store = chunked(&native);
        round_trip(store.as_ref());

        // Short values are stored as before, so existing credentials still read.
        store.set("access-token", "short").unwrap();
        assert_eq!(
            native.get("access-token").unwrap().as_deref(),
            Some("short")
        );

        let long = "0123456789".repeat(9) + "tail";
        store.set("takeover-backup:claude", &long).unwrap();
        assert_eq!(
            store.get("takeover-backup:claude").unwrap().as_deref(),
            Some(long.as_str())
        );
        assert_eq!(
            entries(&native)
                .iter()
                .filter(|key| key.starts_with("takeover-backup:claude#part"))
                .count(),
            10
        );

        // Shorter long value: extra parts are removed.
        let shorter = "abcdefghij".repeat(2) + "x";
        store.set("takeover-backup:claude", &shorter).unwrap();
        assert_eq!(
            store.get("takeover-backup:claude").unwrap().as_deref(),
            Some(shorter.as_str())
        );
        assert_eq!(
            entries(&native)
                .iter()
                .filter(|key| key.starts_with("takeover-backup:claude#part"))
                .count(),
            3
        );

        // Back to a short value, then delete: no parts left behind.
        store.set("takeover-backup:claude", "tiny").unwrap();
        assert!(!entries(&native).iter().any(|key| key.contains("#part")));
        store.set("takeover-backup:claude", &long).unwrap();
        store.delete("takeover-backup:claude").unwrap();
        assert_eq!(store.get("takeover-backup:claude").unwrap(), None);
        assert!(!entries(&native)
            .iter()
            .any(|key| key.starts_with("takeover-backup:")));

        // A value that looks like the header is chunked, not mistaken for one.
        store.set("token:1", "aicli-chunked-v1:3").unwrap();
        assert_eq!(
            store.get("token:1").unwrap().as_deref(),
            Some("aicli-chunked-v1:3")
        );
    }

    /// Windows report (2026-11-12): bind, take over clients, quit, start again → "token invalid".
    /// The large takeover backup failed in Credential Manager, the store downgraded to the file,
    /// and the restarted app looked for the access token only in the file.
    #[test]
    fn large_backup_keeps_native_storage_and_the_token_survives_restart() {
        let temp = tempfile::tempdir().unwrap();
        let native = limited(10);
        let backup = "{\"files\":[123,34]}".repeat(40);

        let store = KeyringStore::with_primary(chunked(&native), file_store(&temp));
        store.set("access-token", "synthetic").unwrap();
        store.set("takeover-backup:codex", &backup).unwrap();
        assert_eq!(store.secret_backend(), SecretBackend::Keychain);
        assert!(!store.fallback.path.exists());
        drop(store);

        let restarted = KeyringStore::with_primary(chunked(&native), file_store(&temp));
        assert_eq!(
            restarted.get("access-token").unwrap().as_deref(),
            Some("synthetic")
        );
        assert_eq!(
            restarted.get("takeover-backup:codex").unwrap().as_deref(),
            Some(backup.as_str())
        );
    }

    /// Installs already in the split state (token in the native store, backup in the file) read
    /// the token back after upgrading, and deleting removes both copies.
    #[test]
    fn file_mode_reads_through_to_keys_left_in_the_native_store() {
        let temp = tempfile::tempdir().unwrap();
        let native = limited(10);
        native.set("access-token", "synthetic").unwrap();
        file_store(&temp)
            .set("takeover-backup:codex", "from-file")
            .unwrap();

        let store = KeyringStore::with_primary(Box::new(native.clone()), file_store(&temp));
        assert_eq!(store.secret_backend(), SecretBackend::FileFallback);
        assert_eq!(
            store.get("access-token").unwrap().as_deref(),
            Some("synthetic")
        );
        assert_eq!(
            store.get("takeover-backup:codex").unwrap().as_deref(),
            Some("from-file")
        );
        store.delete("access-token").unwrap();
        assert_eq!(store.get("access-token").unwrap(), None);
        assert_eq!(native.get("access-token").unwrap(), None);
    }

    #[test]
    fn missing_key_does_not_trigger_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let store = KeyringStore {
            primary: Box::<MemoryStore>::default(),
            fallback: file_store(&temp),
            backend: Mutex::new(SecretBackend::Keychain),
            primary_failed: Default::default(),
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
