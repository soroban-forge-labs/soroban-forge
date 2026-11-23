//! # soroban-forge-identity
//!
//! `soroban-forge identity generate|import|list|show|fund|balance|remove|migrate`
//! — manage test keypairs and fund them via friendbot on testnet.
//!
//! Identities are stored as a JSON file at
//! `~/.config/soroban-forge/identities.json`.
//!
//! ## Secret backends (#616)
//!
//! By default the secret key lives in `identities.json` alongside the public
//! key (file backend, mode `0600` on Unix). Setting
//! `[identity] backend = "keychain"` in `forge.toml` (or user config) moves
//! the secret to the OS keychain (macOS Keychain, Windows Credential Manager,
//! Secret Service on Linux); the JSON file then holds only the name and
//! public key. `identity migrate --to-keychain` / `--to-file` switches an
//! existing store between the two.

mod secret;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use clap::{Arg, ArgMatches, Command};
use serde::{Deserialize, Serialize};
use soroban_forge_core::{ForgeContext, ForgeError, ForgePlugin, Result};

pub use secret::{FileStore, MemoryStore, SecretBackend, SecretStore};
#[cfg(feature = "keychain")]
pub use secret::KeychainStore;
#[cfg(not(feature = "keychain"))]
pub use secret::KeychainStoreDisabled as KeychainStore;

/// A stored test identity.
///
/// `secret_key` is `None` when the backend is the OS keychain — the JSON file
/// then contains only the name and public key. When `None`, callers must use
/// [`resolve_secret`] to obtain the secret from the active backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub public_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_key: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub funded_on: Vec<String>,
}

/// The on-disk identity store.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct IdentityStore {
    #[serde(default)]
    pub identities: BTreeMap<String, Identity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

/// Return the path to the identity store file.
/// `~/.config/soroban-forge/identities.json`
pub fn store_path() -> Result<PathBuf> {
    let config_dir = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(dirs::config_dir)
        .ok_or_else(|| {
            ForgeError::Other("could not determine user config directory".into())
        })?;
    Ok(config_dir.join("soroban-forge").join("identities.json"))
}

/// Load the identity store from disk, or return a default empty one.
pub fn load_store(path: &PathBuf) -> Result<IdentityStore> {
    if !path.is_file() {
        return Ok(IdentityStore::default());
    }
    let raw = std::fs::read_to_string(path)
        .map_err(ForgeError::io(format!("reading {}", path.display())))?;
    serde_json::from_str(&raw).map_err(|e| ForgeError::Config {
        path: path.clone(),
        message: e.to_string(),
    })
}

/// Save the identity store to disk, creating parent directories as needed.
/// Writes through [`soroban_forge_core::atomic::write_atomic`] (#470) and
/// applies mode `0600` on Unix.
pub fn save_store(path: &PathBuf, store: &IdentityStore) -> Result<()> {
    let json = serde_json::to_string_pretty(store)
        .map_err(|e| ForgeError::Other(format!("serializing identity store: {e}")))?;
    #[cfg(unix)]
    let result = soroban_forge_core::atomic::write_atomic_with_mode(path, &json, 0o600);
    #[cfg(not(unix))]
    let result = soroban_forge_core::atomic::write_atomic(path, &json);
    result.map_err(|e| ForgeError::Other(format!("writing {}: {e}", path.display())))
}

/// Read the store, let `f` mutate it, and save it back — all with the store
/// lock held.
pub fn update_store<T>(
    path: &PathBuf,
    f: impl FnOnce(&mut IdentityStore) -> Result<T>,
) -> Result<T> {
    soroban_forge_core::lock::with_lock(path, || {
        let mut store = load_store(path)?;
        let outcome = f(&mut store)?;
        save_store(path, &store)?;
        Ok(outcome)
    })
}

// ---------------------------------------------------------------------------
// Backend selection
// ---------------------------------------------------------------------------

/// Read the active secret backend from `ctx.config`.
///
/// `ctx.config` is `None` when no `forge.toml` exists — that is the historical
/// file backend. An unrecognized value is a hard error (see
/// [`SecretBackend::parse`]) so a typo cannot silently downgrade to plaintext.
pub fn active_backend(ctx: &ForgeContext) -> Result<SecretBackend> {
    let raw = ctx
        .config
        .as_ref()
        .and_then(|config| config.identity.backend.as_deref());
    SecretBackend::parse(raw)
}

/// Instantiate the [`SecretStore`] for `backend`.
///
/// The keychain backend requires the `keychain` feature (on by default); with
/// the feature off it returns a clear "not compiled in" error rather than
/// silently falling back to the file backend.
pub fn secret_store(backend: SecretBackend) -> Box<dyn SecretStore> {
    match backend {
        SecretBackend::File => Box::new(FileStore),
        SecretBackend::Keychain => Box::new(KeychainStore::new()),
    }
}

/// Read the secret for `name` from `store`.
///
/// In file mode the secret lives on [`Identity::secret_key`]; in keychain
/// mode it lives in the OS keychain. Callers should prefer this over reading
/// `secret_key` directly.
pub fn resolve_secret(
    name: &str,
    store: &IdentityStore,
    backend: SecretBackend,
) -> Result<String> {
    match backend {
        SecretBackend::File => store
            .identities
            .get(name)
            .ok_or_else(|| identity_not_found(name))?
            .secret_key
            .clone()
            .ok_or_else(|| {
                ForgeError::InvalidArgument(format!(
                    "identity `{name}` has no stored secret; it was migrated to the OS keychain. \
                     Set `[identity] backend = \"keychain\"` or run `soroban-forge identity migrate --to-file`."
                ))
            }),
        SecretBackend::Keychain => secret_store(backend).get(name),
    }
}

/// Read the secret for `name` for handing off to `stellar` (`deploy`,
/// `invoke`). Same as [`resolve_secret`] but loads the store itself.
///
/// This is the entry point the `deploy` and `invoke` crates use (issue #616,
/// "deploy/invoke key hand-off"). With `backend = "keychain"` it returns the
/// secret fetched from the OS keychain so callers can pass `--source <S...>`
/// to `stellar`; with `"file"` the caller should keep passing the identity
/// *name* (see the `--source` handling in `deploy`/`invoke`).
pub fn resolve_secret_for_handoff(name: &str, ctx: &ForgeContext) -> Result<String> {
    let backend = active_backend(ctx)?;
    let path = store_path()?;
    let store = load_store(&path)?;
    resolve_secret(name, &store, backend)
}

fn identity_not_found(name: &str) -> ForgeError {
    ForgeError::InvalidArgument(format!(
        "identity `{name}` not found (use `soroban-forge identity list` to see available identities)"
    ))
}

// ---------------------------------------------------------------------------
// Keypair generation / derivation
// ---------------------------------------------------------------------------

/// Generate a new keypair and store it under `name` (file backend), returning
/// the stored identity. Used by unit tests and `generate`.
pub fn generate_identity(path: &PathBuf, name: &str, force: bool) -> Result<Identity> {
    generate_identity_with(path, name, force, SecretBackend::File, || {
        Ok(generate_keypair())
    })
}

/// Derive and store a Stellar identity from a BIP-39 mnemonic (file backend).
pub fn generate_identity_from_mnemonic(
    path: &PathBuf,
    name: &str,
    force: bool,
    mnemonic: &str,
    index: u32,
) -> Result<Identity> {
    generate_identity_with(path, name, force, SecretBackend::File, || {
        derive_keypair_from_mnemonic(mnemonic, index)
    })
}

/// Generate into an explicit backend. The returned `Identity.secret_key` is
/// `None` in keychain mode — the secret was written to the keychain.
pub fn generate_identity_into(
    path: &PathBuf,
    name: &str,
    force: bool,
    backend: SecretBackend,
    mnemonic: Option<(&str, u32)>,
) -> Result<Identity> {
    generate_identity_with(path, name, force, backend, || match mnemonic {
        Some((phrase, index)) => derive_keypair_from_mnemonic(phrase, index),
        None => Ok(generate_keypair()),
    })
}

fn generate_identity_with(
    path: &PathBuf,
    name: &str,
    force: bool,
    backend: SecretBackend,
    derive: impl FnOnce() -> Result<(String, String)>,
) -> Result<Identity> {
    let store_impl = secret_store(backend);
    update_store(path, |store| {
        if store.identities.contains_key(name) && !force {
            return Err(ForgeError::AlreadyExists(PathBuf::from(name)));
        }
        let (public_key, secret_key) = derive()?;
        let stored_secret = match backend {
            SecretBackend::File => Some(secret_key.clone()),
            SecretBackend::Keychain => {
                store_impl.set(name, &secret_key)?;
                None
            }
        };
        let identity = Identity {
            public_key,
            secret_key: stored_secret,
            funded_on: Vec::new(),
        };
        store.identities.insert(name.to_string(), identity.clone());
        Ok(identity)
    })
}

/// Remove `name` from the store and (in keychain mode) from the keychain.
pub fn remove_identity(path: &PathBuf, name: &str, backend: SecretBackend) -> Result<()> {
    let store_impl = secret_store(backend);
    update_store(path, |store| {
        if store.identities.remove(name).is_none() {
            return Err(identity_not_found(name));
        }
        if store.default.as_deref() == Some(name) {
            store.default = None;
        }
        if backend == SecretBackend::Keychain {
            store_impl.delete(name)?;
        }
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// forge.toml identity section
// ---------------------------------------------------------------------------

/// Persist the active identity choice into `forge.toml` in `cwd`.
pub fn write_identity_to_forge_toml(cwd: &std::path::Path, name: &str) -> Result<()> {
    let path = cwd.join("forge.toml");

    let existing = if path.is_file() {
        std::fs::read_to_string(&path)
            .map_err(ForgeError::io(format!("reading {}", path.display())))?
    } else {
        String::new()
    };

    let updated = patch_forge_toml_identity(&existing, name);
    std::fs::write(&path, updated)
        .map_err(ForgeError::io(format!("writing {}", path.display())))
}

/// Patch (or append) the `[identity]` section in raw `forge.toml` text.
pub fn patch_forge_toml_identity(existing: &str, name: &str) -> String {
    use toml_edit::{value, DocumentMut, Item, Table};

    let mut doc = existing
        .parse::<DocumentMut>()
        .unwrap_or_else(|_| DocumentMut::new());

    match doc.get_mut("identity") {
        Some(Item::Table(table)) => {
            table["default"] = value(name);
        }
        Some(Item::Value(toml_edit::Value::InlineTable(inline))) => {
            inline.insert("default", name.into());
        }
        _ => {
            let mut table = Table::new();
            table["default"] = value(name);
            doc["identity"] = Item::Table(table);
        }
    }

    let mut result = doc.to_string();
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

// ---------------------------------------------------------------------------
// Keypair generation / validation
// ---------------------------------------------------------------------------

/// Generate a new Stellar keypair and return `(public_key, secret_key)` as
/// Stellar-encoded strings (`G...` / `S...`).
pub fn generate_keypair() -> (String, String) {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    use stellar_strkey::ed25519::{PrivateKey, PublicKey};

    let signing_key = SigningKey::generate(&mut OsRng);
    let seed = signing_key.to_bytes();
    let pubkey = signing_key.verifying_key().to_bytes();

    let public = PublicKey(pubkey).to_string();
    let secret = PrivateKey(seed).to_string();
    (public, secret)
}

/// Derive a Stellar keypair from a BIP-39 mnemonic and account index.
pub fn derive_keypair_from_mnemonic(mnemonic: &str, index: u32) -> Result<(String, String)> {
    use bip39::Mnemonic;
    use ed25519_dalek::SigningKey;
    use hmac::{Hmac, Mac};
    use sha2::Sha512;
    use stellar_strkey::ed25519::{PrivateKey, PublicKey};

    const HARDENED: u32 = 1 << 31;
    if index >= HARDENED {
        return Err(ForgeError::InvalidArgument(
            "mnemonic derivation index must be less than 2^31".into(),
        ));
    }

    let phrase = Mnemonic::parse(mnemonic.trim()).map_err(|_| {
        ForgeError::InvalidArgument("invalid BIP-39 mnemonic phrase".into())
    })?;
    let seed = phrase.to_seed("");

    type HmacSha512 = Hmac<Sha512>;
    let mut master_mac = HmacSha512::new_from_slice(b"ed25519 seed")
        .map_err(|_| ForgeError::Other("could not initialize mnemonic derivation".into()))?;
    master_mac.update(&seed);
    let master = master_mac.finalize().into_bytes();

    let mut private_key = [0u8; 32];
    private_key.copy_from_slice(&master[..32]);
    let mut chain_code = [0u8; 32];
    chain_code.copy_from_slice(&master[32..]);

    for child_index in [44 | HARDENED, 148 | HARDENED, index | HARDENED] {
        let mut child_data = Vec::with_capacity(37);
        child_data.push(0);
        child_data.extend_from_slice(&private_key);
        child_data.extend_from_slice(&child_index.to_be_bytes());

        let mut child_mac = HmacSha512::new_from_slice(&chain_code)
            .map_err(|_| ForgeError::Other("could not initialize mnemonic derivation".into()))?;
        child_mac.update(&child_data);
        let child = child_mac.finalize().into_bytes();
        private_key.copy_from_slice(&child[..32]);
        chain_code.copy_from_slice(&child[32..]);
    }

    let signing_key = SigningKey::from_bytes(&private_key);
    let public_key = PublicKey(signing_key.verifying_key().to_bytes()).to_string();
    let secret_key = PrivateKey(private_key).to_string();
    Ok((public_key, secret_key))
}

/// Derive the public key (`G...`) from a Stellar secret key (`S...`) (#421).
pub fn public_key_from_secret(secret: &str) -> Result<String> {
    use ed25519_dalek::SigningKey;
    use stellar_strkey::ed25519::{PrivateKey, PublicKey};

    let seed = PrivateKey::from_string(secret.trim()).map_err(|_| {
        ForgeError::InvalidArgument(
            "not a valid Stellar secret key: expected a 56-character strkey starting with `S`".into(),
        )
    })?;
    let signing_key = SigningKey::from_bytes(&seed.0);
    Ok(PublicKey(signing_key.verifying_key().to_bytes()).to_string())
}

/// Add an existing keypair to `store` under `name` (file backend, #421).
pub fn import_identity(
    store: &mut IdentityStore,
    name: &str,
    secret: &str,
    force: bool,
) -> Result<Identity> {
    import_identity_into(store, name, secret, force, SecretBackend::File)
}

/// Add an existing keypair into an explicit backend. In keychain mode the
/// secret goes to the keychain and `Identity.secret_key` stays `None`.
pub fn import_identity_into(
    store: &mut IdentityStore,
    name: &str,
    secret: &str,
    force: bool,
    backend: SecretBackend,
) -> Result<Identity> {
    if store.identities.contains_key(name) && !force {
        return Err(ForgeError::AlreadyExists(PathBuf::from(name)));
    }
    let public_key = public_key_from_secret(secret)?;
    let trimmed = secret.trim().to_string();
    let stored_secret = match backend {
        SecretBackend::File => Some(trimmed),
        SecretBackend::Keychain => {
            secret_store(backend).set(name, &trimmed)?;
            None
        }
    };
    let identity = Identity {
        public_key,
        secret_key: stored_secret,
        funded_on: Vec::new(),
    };
    store.identities.insert(name.to_string(), identity.clone());
    Ok(identity)
}

// ---------------------------------------------------------------------------
// Display
// ---------------------------------------------------------------------------

/// Text shown by `identity show` (#422).
pub fn format_show(name: &str, identity: &Identity, secret: Option<&str>) -> String {
    let secret = match secret {
        Some(s) => s,
        None => "[redacted — pass --reveal-secret to reveal]",
    };
    format!(
        "identity `{name}`\n  public key: {}\n  secret key: {secret}\n",
        identity.public_key
    )
}

/// JSON shown by `identity show --json` (#422); `secret_key` only on request.
pub fn show_json(name: &str, identity: &Identity, secret: Option<&str>) -> serde_json::Value {
    let mut report = serde_json::json!({
        "name": name,
        "public_key": identity.public_key,
    });
    if let Some(secret) = secret {
        report["secret_key"] = serde_json::Value::String(secret.to_string());
    }
    report
}

/// Read a secret from the first line of `reader` (`identity import --secret-stdin`).
fn read_secret_line(mut reader: impl std::io::BufRead) -> Result<String> {
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(ForgeError::io("reading secret key from stdin"))?;
    let secret = line.trim().to_string();
    if secret.is_empty() {
        return Err(ForgeError::InvalidArgument(
            "no secret key on stdin (pipe one in, e.g. `echo \"$STELLAR_SECRET\" | soroban-forge identity import ci --secret-stdin`)".into(),
        ));
    }
    Ok(secret)
}

// ---------------------------------------------------------------------------
// Friendbot / balance (unchanged)
// ---------------------------------------------------------------------------

fn http_get(
    url: &str,
    timeout: Option<Duration>,
) -> std::result::Result<ureq::Response, ureq::Error> {
    let mut request = ureq::get(url);
    if let Some(timeout) = timeout {
        request = request.timeout(timeout);
    }
    request.call()
}

fn friendbot_host_for(passphrase: &str) -> Option<&'static str> {
    if passphrase.contains("Public Global Stellar Network") {
        None
    } else if passphrase.contains("Future Network") {
        Some("https://friendbot-futurenet.stellar.org")
    } else if passphrase.contains("Test SDF Network") {
        Some("https://friendbot.stellar.org")
    } else {
        None
    }
}

pub fn friendbot_url(public_key: &str, network_passphrase: Option<&str>) -> Option<String> {
    let passphrase = network_passphrase?;
    let host = friendbot_host_for(passphrase)?;
    Some(format!("{host}/?addr={public_key}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FundingOutcome {
    Funded { balance_xlm: String },
    AlreadyFunded { balance_xlm: Option<String> },
}

impl FundingOutcome {
    fn status(&self) -> &'static str {
        match self {
            Self::Funded { .. } => "funded",
            Self::AlreadyFunded { .. } => "already_funded",
        }
    }

    fn balance_xlm(&self) -> Option<&str> {
        match self {
            Self::Funded { balance_xlm } => Some(balance_xlm),
            Self::AlreadyFunded { balance_xlm } => balance_xlm.as_deref(),
        }
    }
}

pub fn fund_friendbot(
    public_key: &str,
    network_passphrase: Option<&str>,
    timeout: Option<Duration>,
) -> Result<FundingOutcome> {
    let Some(passphrase) = network_passphrase else {
        return Err(ForgeError::InvalidArgument(
            "friendbot funding needs a known network passphrase; \
             set a network with `soroban-forge network use <name>`"
                .into(),
        ));
    };
    if passphrase.contains("Public Global Stellar Network") {
        return Err(ForgeError::InvalidArgument(
            "friendbot funding is only available on testnet/futurenet, not mainnet".into(),
        ));
    }

    let Some(url) = friendbot_url(public_key, network_passphrase) else {
        return Err(ForgeError::InvalidArgument(format!(
            "no friendbot is known for the network passphrase {passphrase:?}; \
             friendbot funding supports testnet and futurenet only"
        )));
    };

    log::debug!("requesting friendbot: {url}");
    let response = http_get(&url, timeout).map_err(|e| {
        ForgeError::Other(format!(
            "friendbot request failed: {e}\n  \
             hint: check your network connection, or the account may already be funded"
        ))
    })?;
    let body = response
        .into_string()
        .map_err(|e| ForgeError::Other(format!("reading friendbot response: {e}")))?;

    parse_friendbot_response(&body)
}

fn parse_friendbot_response(body: &str) -> Result<FundingOutcome> {
    let parsed: Option<serde_json::Value> = serde_json::from_str(body).ok();
    let lower_body = body.to_ascii_lowercase();
    let already_funded = lower_body.contains("already funded")
        || lower_body.contains("already been funded")
        || parsed
            .as_ref()
            .and_then(|value| value.get("already_funded"))
            .and_then(serde_json::Value::as_bool)
            == Some(true)
        || parsed
            .as_ref()
            .and_then(|value| value.get("status"))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|status| status.eq_ignore_ascii_case("already_funded"));

    if already_funded {
        return Ok(FundingOutcome::AlreadyFunded {
            balance_xlm: parsed.as_ref().and_then(parse_native_balance_value),
        });
    }

    if let Some(balance_xlm) = parsed.as_ref().and_then(parse_native_balance_value) {
        return Ok(FundingOutcome::Funded { balance_xlm });
    }

    let detail = parsed
        .as_ref()
        .and_then(|value| value.get("detail").or_else(|| value.get("message")))
        .and_then(serde_json::Value::as_str)
        .unwrap_or(body.trim());
    Err(ForgeError::Other(format!(
        "friendbot returned a success response without confirming funding: {detail}"
    )))
}

fn parse_native_balance(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    parse_native_balance_value(&v)
}

fn parse_native_balance_value(value: &serde_json::Value) -> Option<String> {
    let balances = value.get("balances")?.as_array()?;
    for b in balances {
        if b.get("asset_type")?.as_str()? == "native" {
            let bal = b.get("balance")?.as_str()?;
            return Some(bal.to_string());
        }
    }
    None
}

fn horizon_url_for_passphrase(passphrase: &str) -> Option<&'static str> {
    if passphrase.contains("Future Network") {
        Some("https://horizon-futurenet.stellar.org")
    } else if passphrase.contains("Test SDF Network") {
        Some("https://horizon-testnet.stellar.org")
    } else if passphrase.contains("Public Global Stellar Network") {
        Some("https://horizon.stellar.org")
    } else {
        None
    }
}

fn fetch_native_balance(
    public_key: &str,
    horizon_url: &str,
    timeout: Option<Duration>,
) -> Result<String> {
    let url = format!("{}/accounts/{public_key}", horizon_url.trim_end_matches('/'));
    let response = http_get(&url, timeout).map_err(|error| match error {
        ureq::Error::Status(404, _) => ForgeError::InvalidArgument(format!(
            "identity account `{public_key}` does not exist on the selected network"
        )),
        error => ForgeError::Other(format!("could not query identity balance: {error}")),
    })?;
    let body = response
        .into_string()
        .map_err(|error| ForgeError::Other(format!("reading Horizon response: {error}")))?;
    parse_native_balance(&body).ok_or_else(|| {
        ForgeError::Other("Horizon response did not contain a native XLM balance".into())
    })
}

fn resolve_balance_network_name<'a>(
    cli_network: Option<&'a str>,
    configured_network: Option<&'a str>,
    stored_default: Option<&'a str>,
) -> &'a str {
    cli_network
        .or(configured_network)
        .or(stored_default)
        .unwrap_or("testnet")
}

/// Format the identity list for display.
pub fn format_list(store: &IdentityStore) -> String {
    if store.identities.is_empty() {
        return "no identities stored. Use `soroban-forge identity generate <name>` to create one.\n".to_string();
    }
    let mut out = String::from("stored identities:\n\n");
    let name_width = store.identities.keys().map(|k| k.len()).max().unwrap_or(0);
    for (name, id) in &store.identities {
        let funded_suffix = if id.funded_on.is_empty() {
            String::new()
        } else {
            format!("  funded: {}", id.funded_on.join(", "))
        };
        out.push_str(&format!(
            "  {:<width$}  {}{}\n",
            name,
            id.public_key,
            funded_suffix,
            width = name_width
        ));
    }
    out
}

// ---------------------------------------------------------------------------
// Migration
// ---------------------------------------------------------------------------

/// Move every stored secret from `from` to `to`, then rewrite the store to
/// match the target backend.
///
/// Steps, all under the store lock:
/// 1. Load the store.
/// 2. For every identity, read its secret from `from` (file: the JSON field;
///    keychain: the keychain entry) and write it into `to`.
/// 3. Update `Identity.secret_key` so it is present iff `to == File`.
/// 4. On `to == Keychain`, delete the keychain entry for identities whose
///    file secret was replaced (i.e. none — the source is the JSON file).
///    On `to == File`, delete the keychain entry so a stray copy cannot
///    outlive the migration.
/// 5. Save the store.
///
/// Returns the list of migrated identity names.
pub fn migrate_store(path: &PathBuf, from: SecretBackend, to: SecretBackend) -> Result<Vec<String>> {
    if from == to {
        return Ok(Vec::new());
    }
    let from_store = secret_store(from);
    let to_store = secret_store(to);
    update_store(path, |store| {
        let mut migrated = Vec::new();
        for (name, identity) in store.identities.iter_mut() {
            let secret = match from {
                SecretBackend::File => identity.secret_key.clone().ok_or_else(|| {
                    ForgeError::InvalidArgument(format!(
                        "identity `{name}` has no secret in file backend to migrate"
                    ))
                })?,
                SecretBackend::Keychain => from_store.get(name)?,
            };
            to_store.set(name, &secret)?;
            identity.secret_key = match to {
                SecretBackend::File => Some(secret),
                SecretBackend::Keychain => None,
            };
            migrated.push(name.clone());
        }
        // Drop the old copy in the keychain when moving *into* the file
        // backend, so a stray keychain entry cannot outlive the migration.
        if to == SecretBackend::File {
            for name in &migrated {
                from_store.delete(name)?;
            }
        }
        Ok(migrated)
    })
}

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// The `identity` subcommand.
pub struct IdentityPlugin;

impl ForgePlugin for IdentityPlugin {
    fn name(&self) -> &'static str {
        "identity"
    }

    fn command(&self) -> Command {
        Command::new("identity")
            .about("Manage test keypairs and fund them via friendbot on testnet")
            .subcommand_required(true)
            .subcommand(
                Command::new("generate")
                    .about("Generate a new Stellar test keypair")
                    .arg(
                        Arg::new("name")
                            .help("Name for the identity (e.g. alice, deployer)")
                            .required(true),
                    )
                    .arg(
                        Arg::new("force")
                            .long("force")
                            .action(clap::ArgAction::SetTrue)
                            .help("Overwrite an existing identity with the same name"),
                    )
                    .arg(
                        Arg::new("from-mnemonic")
                            .long("from-mnemonic")
                            .value_name("MNEMONIC")
                            .help("Derive a reproducible identity from a BIP-39 mnemonic"),
                    )
                    .arg(
                        Arg::new("index")
                            .long("index")
                            .value_name("N")
                            .value_parser(clap::value_parser!(u32))
                            .requires("from-mnemonic")
                            .help("Hardened Stellar account derivation index (default: 0)"),
                    )
                    .arg(
                        Arg::new("show-secret")
                            .long("show-secret")
                            .action(clap::ArgAction::SetTrue)
                            .help("Print the secret key in the output (omitted by default for safety)"),
                    ),
            )
            .subcommand(
                Command::new("import")
                    .about("Register an existing Stellar keypair under a name, from its secret key")
                    .long_about(
                        "Store an existing keypair (e.g. a CI secret or a key generated elsewhere). \
                         The public key is derived from the secret, which must be a valid `S...` strkey. \
                         Prefer --secret-stdin over --secret so the key does not end up in shell history.",
                    )
                    .arg(
                        Arg::new("name")
                            .help("Name for the identity (e.g. ci, deployer)")
                            .required(true),
                    )
                    .arg(
                        Arg::new("secret")
                            .long("secret")
                            .value_name("SECRET_KEY")
                            .help("The secret key (S...) to import")
                            .conflicts_with("secret-stdin")
                            .required_unless_present("secret-stdin"),
                    )
                    .arg(
                        Arg::new("secret-stdin")
                            .long("secret-stdin")
                            .action(clap::ArgAction::SetTrue)
                            .help("Read the secret key from the first line of stdin instead"),
                    )
                    .arg(
                        Arg::new("force")
                            .long("force")
                            .action(clap::ArgAction::SetTrue)
                            .help("Overwrite an existing identity with the same name"),
                    ),
            )
            .subcommand(
                Command::new("show")
                    .about("Show a stored identity (secret key only with --reveal-secret)")
                    .arg(
                        Arg::new("name")
                            .help("Name of the identity to show")
                            .required(true),
                    )
                    .arg(
                        Arg::new("reveal-secret")
                            .long("reveal-secret")
                            .action(clap::ArgAction::SetTrue)
                            .help("Also print the stored secret key (explicit opt-in; avoid in shared terminals and CI logs)"),
                    ),
            )
            .subcommand(
                Command::new("list")
                    .about("List all stored identities (public keys only)")
                    .arg(
                        Arg::new("show-secret")
                            .long("show-secret")
                            .action(clap::ArgAction::SetTrue)
                            .help("Also print secret keys (use with care in shared terminals)"),
                    ),
            )
            .subcommand(
                Command::new("fund")
                    .about("Fund a stored identity via Stellar testnet friendbot")
                    .arg(
                        Arg::new("name")
                            .help("Name of the identity to fund")
                            .required(true),
                    )
                    .arg(
                        Arg::new("network-passphrase")
                            .long("network-passphrase")
                            .value_name("PASSPHRASE")
                            .help("Network passphrase (used to refuse mainnet funding)"),
                    ),
            )
            .subcommand(
                Command::new("balance")
                    .about("Show a stored identity's native XLM balance")
                    .arg(
                        Arg::new("name")
                            .help("Name of the identity to check")
                            .required(true),
                    )
                    .arg(
                        Arg::new("network")
                            .long("network")
                            .short('n')
                            .value_name("NETWORK")
                            .help("Network to query [default: configured network or testnet]"),
                    ),
            )
            .subcommand(
                Command::new("remove")
                    .about("Remove a stored identity")
                    .arg(
                        Arg::new("name")
                            .help("Name of the identity to remove")
                            .required(true),
                    ),
            )
            .subcommand(
                Command::new("use")
                    .about("Select a default identity")
                    .arg(
                        Arg::new("name")
                            .help("Name of the identity to set as default")
                            .required(true),
                    ),
            )
            .subcommand(
                Command::new("migrate")
                    .about("Move stored secrets between the file and OS-keychain backends")
                    .arg(
                        Arg::new("to-keychain")
                            .long("to-keychain")
                            .action(clap::ArgAction::SetTrue)
                            .conflicts_with("to-file")
                            .help("Move every stored secret into the OS keychain"),
                    )
                    .arg(
                        Arg::new("to-file")
                            .long("to-file")
                            .action(clap::ArgAction::SetTrue)
                            .conflicts_with("to-keychain")
                            .help("Move every stored secret back into identities.json"),
                    )
                    .arg_required_else_help(true),
            )
    }

    fn run(&self, matches: &ArgMatches, ctx: &ForgeContext) -> Result<()> {
        let path = store_path()?;
        let backend = active_backend(ctx)?;

        match matches.subcommand() {
            Some(("generate", sub)) => {
                let name = sub.get_one::<String>("name").unwrap();
                let force = sub.get_flag("force");
                let show_secret = sub.get_flag("show-secret");
                let mnemonic = sub
                    .get_one::<String>("from-mnemonic")
                    .map(|m| (m.as_str(), sub.get_one::<u32>("index").copied().unwrap_or(0)));
                let identity =
                    generate_identity_into(&path, name, force, backend, mnemonic)?;

                let revealed = if show_secret {
                    Some(resolve_secret(name, &load_store(&path)?, backend)?)
                } else {
                    None
                };

                if ctx.json {
                    let mut report = serde_json::json!({
                        "name": name,
                        "public_key": identity.public_key,
                        "backend": backend.as_str(),
                    });
                    report["secret_key"] = serde_json::Value::String(match revealed.as_deref() {
                        Some(s) => s.to_string(),
                        None => "[redacted — pass --reveal-secret to reveal]".into(),
                    });
                    println!("{}", serde_json::to_string_pretty(&report).unwrap());
                } else if !ctx.quiet {
                    println!("generated identity `{name}`");
                    println!("  public key: {}", identity.public_key);
                    println!("  backend: {}", backend.as_str());
                    if let Some(secret) = revealed {
                        println!("  secret key: {secret}");
                    } else {
                        println!("  secret key: [redacted — pass --reveal-secret to reveal]");
                    }
                    println!();
                    println!("fund on testnet: soroban-forge identity fund {name}");
                }
                Ok(())
            }

            Some(("import", sub)) => {
                let name = sub.get_one::<String>("name").unwrap();
                let force = sub.get_flag("force");
                let secret = match sub.get_one::<String>("secret") {
                    Some(secret) => secret.clone(),
                    None => read_secret_line(std::io::stdin().lock())?,
                };
                let identity = update_store(&path, |store| {
                    import_identity_into(store, name, &secret, force, backend)
                })?;

                if ctx.json {
                    let report = serde_json::json!({
                        "name": name,
                        "public_key": identity.public_key,
                        "imported": true,
                        "backend": backend.as_str(),
                    });
                    println!("{}", serde_json::to_string_pretty(&report).unwrap());
                } else if !ctx.quiet {
                    println!("imported identity `{name}`");
                    println!("  public key: {}", identity.public_key);
                    println!("  backend: {}", backend.as_str());
                }
                Ok(())
            }

            Some(("show", sub)) => {
                let name = sub.get_one::<String>("name").unwrap();
                let reveal_secret = sub.get_flag("reveal-secret");
                let store = load_store(&path)?;
                let identity = store.identities.get(name.as_str()).ok_or_else(|| {
                    identity_not_found(name)
                })?;

                let secret = if reveal_secret {
                    Some(resolve_secret(name, &store, backend)?)
                } else {
                    None
                };

                if ctx.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&show_json(name, identity, secret.as_deref())).unwrap()
                    );
                } else if !ctx.quiet || reveal_secret {
                    print!("{}", format_show(name, identity, secret.as_deref()));
                }
                Ok(())
            }

            Some(("list", sub)) => {
                let show_secret = sub.get_flag("show-secret");
                let store = load_store(&path)?;
                if ctx.json {
                    if show_secret {
                        let mut out = serde_json::Map::new();
                        for (name, id) in &store.identities {
                            let secret = resolve_secret(name, &store, backend)?;
                            out.insert(
                                name.clone(),
                                serde_json::json!({
                                    "public_key": id.public_key,
                                    "funded_on": id.funded_on,
                                    "secret_key": secret,
                                }),
                            );
                        }
                        println!("{}", serde_json::to_string_pretty(&out).unwrap());
                    } else {
                        let public_only: BTreeMap<&str, serde_json::Value> = store
                            .identities
                            .iter()
                            .map(|(k, v)| {
                                (
                                    k.as_str(),
                                    serde_json::json!({
                                        "public_key": v.public_key,
                                        "funded_on": v.funded_on,
                                        "backend": backend.as_str(),
                                    }),
                                )
                            })
                            .collect();
                        println!("{}", serde_json::to_string_pretty(&public_only).unwrap());
                    }
                } else if !ctx.quiet {
                    if show_secret && !store.identities.is_empty() {
                        println!("stored identities:\n");
                        let name_width = store.identities.keys().map(|k| k.len()).max().unwrap_or(0);
                        for (name, id) in &store.identities {
                            let secret = resolve_secret(name, &store, backend)?;
                            let funded_suffix = if id.funded_on.is_empty() {
                                String::new()
                            } else {
                                format!("  funded: {}", id.funded_on.join(", "))
                            };
                            println!(
                                "  {:<width$}  pub: {}  secret: {}{}",
                                name,
                                id.public_key,
                                secret,
                                funded_suffix,
                                width = name_width
                            );
                        }
                    } else {
                        print!("{}", format_list(&store));
                    }
                }
                Ok(())
            }

            Some(("remove", sub)) => {
                let name = sub.get_one::<String>("name").unwrap();
                remove_identity(&path, name, backend)?;

                if ctx.json {
                    println!(
                        "{}",
                        serde_json::json!({ "name": name, "removed": true, "backend": backend.as_str() })
                    );
                } else if !ctx.quiet {
                    println!("removed identity `{name}`");
                }
                Ok(())
            }

            Some(("use", sub)) => {
                let name = sub.get_one::<String>("name").unwrap();
                update_store(&path, |store| {
                    if !store.identities.contains_key(name.as_str()) {
                        return Err(identity_not_found(name));
                    }
                    store.default = Some(name.clone());
                    Ok(())
                })?;

                write_identity_to_forge_toml(&ctx.cwd, name)?;

                if ctx.json {
                    let report = serde_json::json!({ "default": name });
                    println!("{}", serde_json::to_string_pretty(&report).unwrap());
                } else if !ctx.quiet {
                    println!("using `{name}` as the default identity");
                    println!("  recorded in forge.toml and identities.json");
                }
                Ok(())
            }

            Some(("fund", sub)) => {
                if ctx.offline {
                    return Err(ForgeError::InvalidArgument(
                        "friendbot funding is unavailable in offline mode".into(),
                    ));
                }
                let name = sub.get_one::<String>("name").unwrap();
                let network_passphrase = sub
                    .get_one::<String>("network-passphrase")
                    .map(String::as_str);
                let config_passphrase = ctx
                    .config
                    .as_ref()
                    .and_then(|c| c.network.passphrase.as_deref());
                let effective_passphrase = network_passphrase.or(config_passphrase);

                if let Some(passphrase) = effective_passphrase {
                    if passphrase.contains("Public Global Stellar Network") {
                        return Err(ForgeError::InvalidArgument(
                            "friendbot funding is only available on testnet/futurenet, not mainnet".into(),
                        ));
                    }
                }

                let store = load_store(&path)?;
                let pubkey = {
                    let id = store.identities.get(name.as_str()).ok_or_else(|| {
                        identity_not_found(name)
                    })?;
                    id.public_key.clone()
                };

                let net_name = match effective_passphrase {
                    Some(p) if p.contains("Future Network") => "futurenet",
                    _ => "testnet",
                };

                if !ctx.quiet {
                    println!("funding `{name}` ({pubkey}) via {net_name} friendbot...");
                }

                let outcome = fund_friendbot(&pubkey, effective_passphrase, ctx.timeout())?;

                update_store(&path, |store| {
                    if let Some(id) = store.identities.get_mut(name.as_str()) {
                        if !id.funded_on.iter().any(|n| n == net_name) {
                            id.funded_on.push(net_name.to_string());
                        }
                    }
                    Ok(())
                })?;

                if ctx.json {
                    let report = serde_json::json!({
                        "name": name,
                        "public_key": pubkey,
                        "funded": true,
                        "newly_funded": outcome.status() == "funded",
                        "status": outcome.status(),
                        "network": net_name,
                        "balance_xlm": outcome.balance_xlm(),
                    });
                    println!("{}", serde_json::to_string_pretty(&report).unwrap());
                } else if !ctx.quiet {
                    match &outcome {
                        FundingOutcome::Funded { balance_xlm } => {
                            println!("funded `{name}` on {net_name}.");
                            println!("  balance: {balance_xlm} XLM");
                        }
                        FundingOutcome::AlreadyFunded { balance_xlm } => {
                            println!("`{name}` was already funded on {net_name}.");
                            if let Some(balance) = balance_xlm {
                                println!("  balance: {balance} XLM");
                            }
                        }
                    }
                }
                Ok(())
            }

            Some(("balance", sub)) => {
                if ctx.offline {
                    return Err(ForgeError::InvalidArgument(
                        "identity balance is unavailable in offline mode".into(),
                    ));
                }

                let name = sub.get_one::<String>("name").unwrap();
                let network_path = soroban_forge_network::store_path()?;
                let network_store = soroban_forge_network::load_store(&network_path)?;
                let configured_network = ctx
                    .config
                    .as_ref()
                    .and_then(|config| config.network.name.as_deref());
                let network_name = resolve_balance_network_name(
                    sub.get_one::<String>("network").map(String::as_str),
                    configured_network,
                    network_store.default.as_deref(),
                );
                let network = soroban_forge_network::resolve_network(&network_store, network_name)
                    .ok_or_else(|| {
                        ForgeError::InvalidArgument(format!(
                            "network `{network_name}` is not configured"
                        ))
                    })?;
                let passphrase = if sub.get_one::<String>("network").is_none() {
                    ctx.config
                        .as_ref()
                        .and_then(|config| config.network.passphrase.as_deref())
                        .unwrap_or(&network.network_passphrase)
                } else {
                    &network.network_passphrase
                };
                let horizon_url = horizon_url_for_passphrase(passphrase).ok_or_else(|| {
                    ForgeError::InvalidArgument(format!(
                        "identity balance requires a known Horizon endpoint; network `{network_name}` is not supported"
                    ))
                })?;

                let store = load_store(&path)?;
                let identity = store.identities.get(name.as_str()).ok_or_else(|| {
                    identity_not_found(name)
                })?;
                let balance = fetch_native_balance(
                    &identity.public_key,
                    horizon_url,
                    ctx.timeout(),
                )?;

                if ctx.json {
                    let report = serde_json::json!({
                        "name": name,
                        "public_key": identity.public_key,
                        "network": network_name,
                        "balance_xlm": balance,
                    });
                    println!("{}", serde_json::to_string_pretty(&report).unwrap());
                } else if !ctx.quiet {
                    println!("balance for `{name}` on {network_name}: {balance} XLM");
                }
                Ok(())
            }

            Some(("migrate", sub)) => {
                let to = if sub.get_flag("to-keychain") {
                    SecretBackend::Keychain
                } else if sub.get_flag("to-file") {
                    SecretBackend::File
                } else {
                    return Err(ForgeError::InvalidArgument(
                        "pass --to-keychain or --to-file".into(),
                    ));
                };
                let from = match to {
                    SecretBackend::Keychain => SecretBackend::File,
                    SecretBackend::File => SecretBackend::Keychain,
                };
                let migrated = migrate_store(&path, from, to)?;

                if ctx.json {
                    let report = serde_json::json!({
                        "from": from.as_str(),
                        "to": to.as_str(),
                        "migrated": migrated,
                    });
                    println!("{}", serde_json::to_string_pretty(&report).unwrap());
                } else if !ctx.quiet {
                    if migrated.is_empty() {
                        println!("no identities to migrate");
                    } else {
                        println!(
                            "migrated {} identit{} from {} to {}",
                            migrated.len(),
                            if migrated.len() == 1 { "y" } else { "ies" },
                            from.as_str(),
                            to.as_str(),
                        );
                        for name in &migrated {
                            println!("  {name}");
                        }
                    }
                }
                Ok(())
            }

            _ => Err(ForgeError::InvalidArgument(
                "unknown identity subcommand".into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests;
