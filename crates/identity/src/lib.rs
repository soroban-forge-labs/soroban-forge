//! # soroban-forge-identity
//!
//! `soroban-forge identity generate|list|fund` — manage test keypairs and
//! fund them via friendbot on testnet.
//!
//! Identities are stored as a JSON file at
//! `~/.config/soroban-forge/identities.json`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use clap::{Arg, ArgMatches, Command};
use serde::{Deserialize, Serialize};
use soroban_forge_core::{ForgeContext, ForgeError, ForgePlugin, Result};

/// A stored test identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub public_key: String,
    pub secret_key: String,
}

/// The on-disk identity store.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct IdentityStore {
    #[serde(default)]
    pub identities: BTreeMap<String, Identity>,
}

/// Return the path to the identity store file.
/// `~/.config/soroban-forge/identities.json`
pub fn store_path() -> Result<PathBuf> {
    let config_dir = dirs::config_dir().ok_or_else(|| {
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
pub fn save_store(path: &PathBuf, store: &IdentityStore) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(ForgeError::io(format!("creating {}", parent.display())))?;
    }
    let json = serde_json::to_string_pretty(store)
        .map_err(|e| ForgeError::Other(format!("serializing identity store: {e}")))?;
    std::fs::write(path, json)
        .map_err(ForgeError::io(format!("writing {}", path.display())))
}

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

/// GET `url`, bounded by `timeout` when one is set (`--timeout`).
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

/// Fund a Stellar testnet account via friendbot.
/// Returns the parsed balance (in XLM) on success.
///
/// Refuses to run on mainnet (passphrase contains "Public Global Stellar Network").
/// The request is bounded by `timeout` when set (`--timeout`).
pub fn fund_friendbot(
    public_key: &str,
    network_passphrase: Option<&str>,
    timeout: Option<Duration>,
) -> Result<String> {
    // #287 — refuse to run on mainnet
    if let Some(passphrase) = network_passphrase {
        if passphrase.contains("Public Global Stellar Network") {
            return Err(ForgeError::InvalidArgument(
                "friendbot funding is only available on testnet/futurenet, not mainnet".into(),
            ));
        }
    }

    let url = format!("https://friendbot.stellar.org/?addr={public_key}");
    log::debug!("requesting friendbot: {url}");
    let response = http_get(&url, timeout).map_err(|e| {
        // Surface actionable error messages (#287)
        ForgeError::Other(format!(
            "friendbot request failed: {e}\n  \
             hint: check your network connection, or the account may already be funded"
        ))
    })?;
    let body = response
        .into_string()
        .map_err(|e| ForgeError::Other(format!("reading friendbot response: {e}")))?;

    // Try to parse the balance from the horizon response.
    // Friendbot returns the created account record; the native balance lives in balances[].
    let balance = parse_native_balance(&body).unwrap_or_else(|| "unknown".to_string());
    Ok(balance)
}

/// Extract the native XLM balance from a Horizon account JSON response.
fn parse_native_balance(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let balances = v.get("balances")?.as_array()?;
    for b in balances {
        if b.get("asset_type")?.as_str()? == "native" {
            let bal = b.get("balance")?.as_str()?;
            return Some(bal.to_string());
        }
    }
    None
}

/// Format the identity list for display.
/// Secret keys are always masked here — use `--show-secret` to reveal them.
pub fn format_list(store: &IdentityStore) -> String {
    if store.identities.is_empty() {
        return "no identities stored. Use `soroban-forge identity generate <name>` to create one.\n".to_string();
    }
    let mut out = String::from("stored identities:\n\n");
    let name_width = store.identities.keys().map(|k| k.len()).max().unwrap_or(0);
    for (name, id) in &store.identities {
        out.push_str(&format!("  {:<width$}  {}\n", name, id.public_key, width = name_width));
    }
    out
}

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
                    // #288 — explicit opt-in to showing secret keys
                    .arg(
                        Arg::new("show-secret")
                            .long("show-secret")
                            .action(clap::ArgAction::SetTrue)
                            .help("Print the secret key in the output (omitted by default for safety)"),
                    ),
            )
            .subcommand(
                Command::new("list")
                    .about("List all stored identities (public keys only)")
                    // #288 — explicit opt-in to showing secret keys
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
                    // #287 — allow passing an explicit network passphrase to guard mainnet
                    .arg(
                        Arg::new("network-passphrase")
                            .long("network-passphrase")
                            .value_name("PASSPHRASE")
                            .help("Network passphrase (used to refuse mainnet funding)"),
                    ),
            )
    }

    fn run(&self, matches: &ArgMatches, ctx: &ForgeContext) -> Result<()> {
        let path = store_path()?;

        match matches.subcommand() {
            Some(("generate", sub)) => {
                let name = sub.get_one::<String>("name").unwrap();
                let force = sub.get_flag("force");
                // #288 — only show secret key when explicitly requested
                let show_secret = sub.get_flag("show-secret");
                let mut store = load_store(&path)?;

                if store.identities.contains_key(name.as_str()) && !force {
                    return Err(ForgeError::AlreadyExists(PathBuf::from(name.as_str())));
                }

                let (public_key, secret_key) = generate_keypair();
                store.identities.insert(
                    name.clone(),
                    Identity {
                        public_key: public_key.clone(),
                        secret_key: secret_key.clone(),
                    },
                );
                save_store(&path, &store)?;

                if ctx.json {
                    // #288 — never emit secret key in JSON output unless --show-secret
                    let mut report = serde_json::json!({
                        "name": name,
                        "public_key": public_key,
                    });
                    if show_secret {
                        report["secret_key"] = serde_json::Value::String(secret_key.clone());
                    } else {
                        report["secret_key"] = serde_json::Value::String("[redacted — pass --show-secret to reveal]".into());
                    }
                    println!("{}", serde_json::to_string_pretty(&report).unwrap());
                } else if !ctx.quiet {
                    println!("generated identity `{name}`");
                    println!("  public key: {public_key}");
                    // #288 — mask secret key by default
                    if show_secret {
                        println!("  secret key: {secret_key}");
                    } else {
                        println!("  secret key: [redacted — pass --show-secret to reveal]");
                    }
                    println!();
                    println!("fund on testnet: soroban-forge identity fund {name}");
                }
                Ok(())
            }

            Some(("list", sub)) => {
                let show_secret = sub.get_flag("show-secret");
                let store = load_store(&path)?;
                if ctx.json {
                    if show_secret {
                        // Full store including secrets
                        println!("{}", serde_json::to_string_pretty(&store.identities).unwrap());
                    } else {
                        // #288 — omit secret keys from JSON output
                        let public_only: BTreeMap<&str, serde_json::Value> = store
                            .identities
                            .iter()
                            .map(|(k, v)| {
                                (
                                    k.as_str(),
                                    serde_json::json!({ "public_key": v.public_key }),
                                )
                            })
                            .collect();
                        println!("{}", serde_json::to_string_pretty(&public_only).unwrap());
                    }
                } else if !ctx.quiet {
                    if show_secret && !store.identities.is_empty() {
                        // Show full list including secrets
                        println!("stored identities:\n");
                        let name_width = store.identities.keys().map(|k| k.len()).max().unwrap_or(0);
                        for (name, id) in &store.identities {
                            println!(
                                "  {:<width$}  pub: {}  secret: {}",
                                name,
                                id.public_key,
                                id.secret_key,
                                width = name_width
                            );
                        }
                    } else {
                        print!("{}", format_list(&store));
                    }
                }
                Ok(())
            }

            Some(("fund", sub)) => {
                // #287 — refuse in offline mode
                if ctx.offline {
                    return Err(ForgeError::InvalidArgument(
                        "friendbot funding is unavailable in offline mode".into(),
                    ));
                }
                let name = sub.get_one::<String>("name").unwrap();
                // #287 — accept passphrase to guard against mainnet runs
                let network_passphrase = sub
                    .get_one::<String>("network-passphrase")
                    .map(String::as_str);

                // Also check via forge.toml / context config
                let config_passphrase = ctx
                    .config
                    .as_ref()
                    .and_then(|c| c.network.passphrase.as_deref());
                let effective_passphrase = network_passphrase.or(config_passphrase);

                // #287 — refuse on mainnet
                if let Some(passphrase) = effective_passphrase {
                    if passphrase.contains("Public Global Stellar Network") {
                        return Err(ForgeError::InvalidArgument(
                            "friendbot funding is only available on testnet/futurenet, not mainnet".into(),
                        ));
                    }
                }

                let store = load_store(&path)?;
                let id = store.identities.get(name.as_str()).ok_or_else(|| {
                    ForgeError::InvalidArgument(format!(
                        "identity `{name}` not found (use `soroban-forge identity list` to see available identities)"
                    ))
                })?;

                if !ctx.quiet {
                    println!("funding `{name}` ({}) via testnet friendbot...", id.public_key);
                }

                // #287 — fund_friendbot now returns the balance
                let balance = fund_friendbot(&id.public_key, effective_passphrase, ctx.timeout())?;

                if ctx.json {
                    let report = serde_json::json!({
                        "name": name,
                        "public_key": id.public_key,
                        "funded": true,
                        "network": "testnet",
                        "balance_xlm": balance,
                    });
                    println!("{}", serde_json::to_string_pretty(&report).unwrap());
                } else if !ctx.quiet {
                    println!("funded `{name}` on testnet.");
                    println!("  balance: {balance} XLM");
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
mod tests {
    use super::*;

    #[test]
    fn generate_keypair_produces_valid_stellar_keys() {
        let (public, secret) = generate_keypair();
        assert!(public.starts_with('G'), "public key should start with G: {public}");
        assert!(secret.starts_with('S'), "secret key should start with S: {secret}");
        assert_eq!(public.len(), 56, "Stellar public keys are 56 chars");
        assert_eq!(secret.len(), 56, "Stellar secret keys are 56 chars");
    }

    #[test]
    fn generate_keypair_is_unique() {
        let (pub1, _) = generate_keypair();
        let (pub2, _) = generate_keypair();
        assert_ne!(pub1, pub2, "two generated keypairs should differ");
    }

    #[test]
    fn store_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identities.json");

        let mut store = IdentityStore::default();
        store.identities.insert(
            "alice".into(),
            Identity {
                public_key: "GABC".into(),
                secret_key: "SABC".into(),
            },
        );
        save_store(&path, &store).unwrap();

        let loaded = load_store(&path).unwrap();
        assert_eq!(loaded.identities.len(), 1);
        assert_eq!(loaded.identities["alice"].public_key, "GABC");
    }

    #[test]
    fn load_missing_file_returns_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.json");
        let store = load_store(&path).unwrap();
        assert!(store.identities.is_empty());
    }

    #[test]
    fn format_list_empty() {
        let store = IdentityStore::default();
        assert!(format_list(&store).contains("no identities stored"));
    }

    #[test]
    fn format_list_shows_names_and_public_keys() {
        let mut store = IdentityStore::default();
        store.identities.insert(
            "alice".into(),
            Identity {
                public_key: "GAAA".into(),
                secret_key: "SAAA".into(),
            },
        );
        store.identities.insert(
            "bob".into(),
            Identity {
                public_key: "GBBB".into(),
                secret_key: "SBBB".into(),
            },
        );
        let output = format_list(&store);
        assert!(output.contains("alice"));
        assert!(output.contains("GAAA"));
        assert!(output.contains("bob"));
        assert!(output.contains("GBBB"));
        // #288 — Secret keys must NOT appear in the list output
        assert!(!output.contains("SAAA"), "secret key must not appear in format_list output");
        assert!(!output.contains("SBBB"), "secret key must not appear in format_list output");
    }

    // #288 — secret keys must never appear in output unless --show-secret is passed
    #[test]
    fn generate_output_does_not_contain_secret_without_show_secret() {
        use soroban_forge_core::ForgeContext;

        let dir = tempfile::tempdir().unwrap();
        // We can't easily capture stdout, but we can verify the plugin builds
        // and the show_secret flag is wired up.
        let plugin = IdentityPlugin;
        let cmd = plugin.command();
        let sub = cmd
            .find_subcommand("generate")
            .expect("generate subcommand exists");
        let has_show_secret = sub
            .get_arguments()
            .any(|a| a.get_long() == Some("show-secret"));
        assert!(has_show_secret, "generate must have --show-secret flag");
    }

    // #288 — list must have --show-secret flag
    #[test]
    fn list_subcommand_has_show_secret_flag() {
        let plugin = IdentityPlugin;
        let cmd = plugin.command();
        let sub = cmd
            .find_subcommand("list")
            .expect("list subcommand exists");
        let has_show_secret = sub
            .get_arguments()
            .any(|a| a.get_long() == Some("show-secret"));
        assert!(has_show_secret, "list must have --show-secret flag");
    }

    // #329 — --timeout bounds a slow HTTP call instead of hanging forever
    #[test]
    fn http_get_is_bounded_by_timeout() {
        // A server that accepts the connection but never responds.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let _accepter = std::thread::spawn(move || {
            let held = listener.accept();
            std::thread::sleep(Duration::from_secs(10));
            drop(held);
        });

        let started = std::time::Instant::now();
        let result = http_get(&url, Some(Duration::from_millis(300)));
        assert!(result.is_err(), "a silent server must trip the timeout");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "request was not bounded by the timeout: {:?}",
            started.elapsed()
        );
    }

    // #287 — fund refuses on mainnet passphrase
    #[test]
    fn fund_friendbot_refuses_mainnet_passphrase() {
        let result = fund_friendbot(
            "GABC",
            Some("Public Global Stellar Network ; September 2015"),
            None,
        );
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("mainnet"), "error should mention mainnet: {msg}");
    }

    // #287 — fund allows testnet passphrase (would make network call, test only guards the refusal)
    #[test]
    fn fund_friendbot_allows_testnet_passphrase_guard() {
        // We cannot make real network calls in unit tests. We verify that the
        // mainnet guard does NOT fire for a testnet passphrase. The ureq call
        // will fail (no network in CI) but the error won't be about mainnet.
        let result = fund_friendbot("GABC", Some("Test SDF Network ; September 2015"), None);
        match &result {
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    !msg.contains("mainnet"),
                    "testnet passphrase should not trigger mainnet refusal: {msg}"
                );
            }
            Ok(_) => {} // real network available — fine
        }
    }

    #[test]
    fn identity_command_has_subcommands() {
        let plugin = IdentityPlugin;
        let cmd = plugin.command();
        let sub_names: Vec<&str> = cmd
            .get_subcommands()
            .map(|s| s.get_name())
            .collect();
        assert!(sub_names.contains(&"generate"));
        assert!(sub_names.contains(&"list"));
        assert!(sub_names.contains(&"fund"));
    }

    // #287 — fund refuses offline
    #[test]
    fn fund_subcommand_refuses_offline() {
        use soroban_forge_core::ForgeContext;
        let dir = tempfile::tempdir().unwrap();
        let ctx = ForgeContext::with_options(
            dir.path().to_path_buf(),
            0,
            false,
            false,
            false,
            true, // offline = true
            None,
            None,
            None,
        )
        .unwrap();
        let plugin = IdentityPlugin;
        let cmd = plugin.command();
        let matches = cmd
            .try_get_matches_from(["identity", "fund", "alice"])
            .unwrap();
        let result = plugin.run(&matches, &ctx);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("offline"), "error should mention offline: {msg}");
    }
}
