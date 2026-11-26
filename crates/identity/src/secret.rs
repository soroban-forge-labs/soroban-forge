//! Pluggable secret storage for identities (#616).
//!
//! Historically the identity store kept secret keys in plain text inside
//! `identities.json` alongside the public key. File permissions (#423) narrow
//! who can read it, but the file still ends up in backups, synced home
//! directories and support bundles. This module adds an opt-in OS-keychain
//! backend so the JSON store can hold only the name and public key while the
//! secret lives in the platform keychain.
//!
//! The backend is selected by `[identity] backend` in `forge.toml` /
//! user config (`"file"` by default, `"keychain"` to opt in). The two
//! backends differ only in *where* the secret lives; the rest of the crate
//! reads and writes secrets through [`SecretStore`] and never has to know
//! which backend is active.
//!
//! Testing: [`MemoryStore`] is an in-memory implementation used by unit tests
//! so the suite never touches the real OS keychain. The real
//! [`KeychainStore`] is exercised only by `#[ignore]`d integration tests.

use crate::{ForgeError, Result};

/// Which storage backend identity secrets live in.
///
/// Mirrors the `[identity] backend` config value. `File` is the historical
/// behaviour (secret inside `identities.json`); `Keychain` is the opt-in
/// OS-keychain backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SecretBackend {
    /// Secret stored in `identities.json` next to the public key (default).
    #[default]
    File,
    /// Secret stored in the OS keychain (macOS Keychain / Windows Credential
    /// Manager / Secret Service on Linux).
    Keychain,
}

impl SecretBackend {
    /// Parse a `[identity] backend` config value.
    ///
    /// Accepts `"file"` and `"keychain"` case-insensitively. `None` (unset)
    /// yields the default (`File`). Any other string is a configuration
    /// error, so a typo like `"keychian"` fails loudly instead of silently
    /// falling back to plaintext storage.
    pub fn parse(value: Option<&str>) -> Result<Self> {
        match value.map(str::trim).filter(|v| !v.is_empty()) {
            None => Ok(Self::default()),
            Some(v) if v.eq_ignore_ascii_case("file") => Ok(Self::File),
            Some(v) if v.eq_ignore_ascii_case("keychain") => Ok(Self::Keychain),
            Some(other) => Err(ForgeError::InvalidArgument(format!(
                "[identity] backend = {other:?} is not recognized; expected \"file\" or \"keychain\""
            ))),
        }
    }

    /// The config string for this backend, i.e. what `[identity] backend`
    /// would contain.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Keychain => "keychain",
        }
    }
}

/// Storage for identity secret keys.
///
/// Implementations are cheap to construct; every read/write goes straight to
/// the backing store. The trait exists so tests can substitute
/// [`MemoryStore`] and so `deploy`/`invoke` can resolve secrets through the
/// same interface the `identity` commands use.
pub trait SecretStore {
    /// Read the secret for `name`.
    ///
    /// Returns [`ForgeError::InvalidArgument`] when the identity is unknown,
    /// and [`ForgeError::Other`] for backend failures (keychain locked, entry
    /// missing, permission denied, …).
    fn get(&self, name: &str) -> Result<String>;

    /// Store `secret` for `name`, replacing any previous value.
    fn set(&self, name: &str, secret: &str) -> Result<()>;

    /// Remove the secret for `name`. A missing entry is not an error.
    fn delete(&self, name: &str) -> Result<()>;
}

/// The historical backend: the secret lives in `identities.json` alongside
/// the public key.
///
/// This type does **not** touch the JSON store itself — that is the identity
/// crate's job. It is a zero-sized marker that turns "the secret is already
/// in the store" into the [`SecretStore`] interface, so callers that only
/// know about [`SecretStore`] work unchanged in file mode.
#[derive(Debug, Default, Clone, Copy)]
pub struct FileStore;

impl SecretStore for FileStore {
    fn get(&self, name: &str) -> Result<String> {
        Err(ForgeError::InvalidArgument(format!(
            "identity `{name}` has no stored secret in file backend; \
             use `soroban-forge identity migrate --to-keychain` to move it"
        )))
    }

    fn set(&self, _name: &str, _secret: &str) -> Result<()> {
        // No-op: `identities.json` already carries the secret. The identity
        // crate writes it as part of `save_store`.
        Ok(())
    }

    fn delete(&self, _name: &str) -> Result<()> {
        // No-op: removing the identity from `identities.json` removes the
        // secret with it.
        Ok(())
    }
}

/// The opt-in backend: the secret lives in the OS keychain under the service
/// name [`KEYCHAIN_SERVICE`].
///
/// Only compiled when the `keychain` feature is enabled, because `keyring`
/// pulls in platform-specific crates on Linux (`secret-service` speaks D-Bus)
/// that not every CI environment can build.
#[cfg(feature = "keychain")]
pub struct KeychainStore;

#[cfg(feature = "keychain")]
pub const KEYCHAIN_SERVICE: &str = "soroban-forge";

#[cfg(feature = "keychain")]
impl SecretStore for KeychainStore {
    fn get(&self, name: &str) -> Result<String> {
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, name).map_err(keychain_error(name, "open"))?;
        entry
            .get_password()
            .map_err(keychain_error(name, "read"))
    }

    fn set(&self, name: &str, secret: &str) -> Result<()> {
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, name).map_err(keychain_error(name, "open"))?;
        entry
            .set_password(secret)
            .map_err(keychain_error(name, "write"))
    }

    fn delete(&self, name: &str) -> Result<()> {
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, name).map_err(keychain_error(name, "open"))?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            // A missing entry is not a failure — the caller asked for it gone.
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(keychain_error(name, "delete")(e)),
        }
    }
}

#[cfg(feature = "keychain")]
fn keychain_error(
    name: &str,
    action: &'static str,
) -> impl FnOnce(keyring::Error) -> ForgeError + 'static {
    let name = name.to_string();
    move |e| {
        ForgeError::Other(format!(
            "OS keychain could not {action} secret for identity `{name}`: {e}\n  \
             hint: on macOS, unlock the login keychain; on Linux, ensure a Secret Service \
             (gnome-keyring / KWallet) is running; on Windows, the credential manager must be available"
        ))
    }
}

/// In-memory [`SecretStore`] for unit tests.
///
/// Compiles on every platform and never touches the real keychain, so the
/// test suite runs identically on Linux CI (no Secret Service), macOS CI
/// (no keychain prompts) and Windows CI.
#[derive(Debug, Default, Clone)]
pub struct MemoryStore {
    entries: std::sync::Arc<std::sync::Mutex<std::collections::BTreeMap<String, String>>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-seed an entry, for tests that start from a known state.
    pub fn insert(&self, name: impl Into<String>, secret: impl Into<String>) {
        self.entries
            .lock()
            .expect("memory store poisoned")
            .insert(name.into(), secret.into());
    }

    /// Number of entries currently held, for test assertions.
    pub fn len(&self) -> usize {
        self.entries.lock().expect("memory store poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn contains(&self, name: &str) -> bool {
        self.entries
            .lock()
            .expect("memory store poisoned")
            .contains_key(name)
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, name: &str) -> Result<String> {
        self.entries
            .lock()
            .expect("memory store poisoned")
            .get(name)
            .cloned()
            .ok_or_else(|| ForgeError::InvalidArgument(format!("no secret stored for identity `{name}`")))
    }

    fn set(&self, name: &str, secret: &str) -> Result<()> {
        self.entries
            .lock()
            .expect("memory store poisoned")
            .insert(name.to_string(), secret.to_string());
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<()> {
        self.entries
            .lock()
            .expect("memory store poisoned")
            .remove(name);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_parse_accepts_the_two_documented_values_case_insensitively() {
        assert_eq!(SecretBackend::parse(None).unwrap(), SecretBackend::File);
        assert_eq!(SecretBackend::parse(Some("")).unwrap(), SecretBackend::File);
        assert_eq!(SecretBackend::parse(Some("file")).unwrap(), SecretBackend::File);
        assert_eq!(SecretBackend::parse(Some("FILE")).unwrap(), SecretBackend::File);
        assert_eq!(
            SecretBackend::parse(Some("keychain")).unwrap(),
            SecretBackend::Keychain
        );
        assert_eq!(
            SecretBackend::parse(Some("KeyChain")).unwrap(),
            SecretBackend::Keychain
        );
    }

    #[test]
    fn backend_parse_rejects_typos_loudly() {
        let err = SecretBackend::parse(Some("keychian")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("keychian"), "{msg}");
        assert!(msg.contains("\"file\" or \"keychain\""), "{msg}");
    }

    #[test]
    fn backend_as_str_round_trips_through_parse() {
        for backend in [SecretBackend::File, SecretBackend::Keychain] {
            assert_eq!(SecretBackend::parse(Some(backend.as_str())).unwrap(), backend);
        }
    }

    #[test]
    fn memory_store_set_get_delete_round_trip() {
        let store = MemoryStore::new();
        assert!(store.is_empty());
        assert!(store.get("alice").is_err());

        store.set("alice", "SALICE").unwrap();
        assert_eq!(store.get("alice").unwrap(), "SALICE");
        assert!(store.contains("alice"));

        store.set("alice", "SALICE2").unwrap();
        assert_eq!(store.get("alice").unwrap(), "SALICE2");
        assert_eq!(store.len(), 1);

        store.delete("alice").unwrap();
        assert!(!store.contains("alice"));
        // Deleting again is not an error.
        store.delete("alice").unwrap();
    }

    #[test]
    fn memory_store_errors_are_clear_when_the_entry_is_missing() {
        let store = MemoryStore::new();
        let err = store.get("ghost").unwrap_err().to_string();
        assert!(err.contains("no secret stored"), "{err}");
        assert!(err.contains("ghost"), "{err}");
    }

    #[test]
    fn file_store_is_a_no_op_so_it_does_not_fight_identities_json() {
        let store = FileStore;
        assert!(store.set("alice", "SALICE").is_ok());
        assert!(store.delete("alice").is_ok());
        // Reading a secret from file backend is not supported here: the
        // identity crate reads it straight out of the JSON struct.
        assert!(store.get("alice").is_err());
    }
}
