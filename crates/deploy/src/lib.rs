//! # soroban-forge-deploy
//!
//! `soroban-forge deploy` — builds the contract wasm if it hasn't been built
//! yet, then deploys it with the official `stellar contract deploy` and
//! prints the resulting contract ID.
//!
//! Per soroban-forge's "wrap, don't reimplement" rule, both the build and the
//! deploy are done by shelling out to the `stellar` CLI; this module only
//! locates the wasm, decides whether a build is needed, assembles the CLI
//! arguments and extracts the contract ID from the CLI's output.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::time::Duration;

use clap::{Arg, ArgAction, ArgMatches, Command};
use serde::Deserialize;
use soroban_forge_core::{ForgeContext, ForgeError, ForgePlugin, Result};

/// Network used when neither `--network` nor `--rpc-url` is given.
pub const DEFAULT_NETWORK: &str = "testnet";

#[derive(Deserialize)]
struct Manifest {
    package: Package,
}

#[derive(Deserialize)]
struct Package {
    name: String,
}

/// Read `[package].name` out of `dir/Cargo.toml` and return it as a crate
/// name (snake_case), which is what the build output is named after.
///
/// Deliberately duplicated rather than shared with `verify`/`bindings ts`:
/// modules depend only on `soroban-forge-core`, never on each other.
pub fn read_crate_name(dir: &Path) -> Result<String> {
    let manifest_path = dir.join("Cargo.toml");
    if !manifest_path.is_file() {
        return Err(ForgeError::InvalidArgument(format!(
            "{} is not a cargo project (no Cargo.toml)",
            dir.display()
        )));
    }
    let raw = std::fs::read_to_string(&manifest_path).map_err(ForgeError::io(format!(
        "reading {}",
        manifest_path.display()
    )))?;
    let manifest: Manifest = toml::from_str(&raw).map_err(|e| ForgeError::Config {
        path: manifest_path.clone(),
        message: e.to_string(),
    })?;
    Ok(manifest.package.name.replace('-', "_"))
}

/// Default location `stellar contract build` writes its release wasm to.
pub fn locate_wasm(dir: &Path, crate_name: &str) -> PathBuf {
    dir.join("target/wasm32v1-none/release")
        .join(format!("{crate_name}.wasm"))
}

/// How to reach the network, mirroring the `stellar` CLI's own options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkArgs {
    /// A configured network name, e.g. `testnet`.
    pub network: Option<String>,
    /// An explicit RPC endpoint, used instead of a named network.
    pub rpc_url: Option<String>,
    /// Passphrase for the endpoint given by `rpc_url`.
    pub network_passphrase: Option<String>,
}

impl NetworkArgs {
    /// Apply the default: with no network *and* no RPC URL we target
    /// [`DEFAULT_NETWORK`]. An explicit `--rpc-url` alone is left alone, so
    /// the endpoint the user asked for is the one we talk to.
    pub fn resolve(
        network: Option<String>,
        rpc_url: Option<String>,
        network_passphrase: Option<String>,
    ) -> Self {
        let network = match (network, rpc_url.as_ref()) {
            (Some(name), _) => Some(name),
            (None, None) => Some(DEFAULT_NETWORK.to_string()),
            (None, Some(_)) => None,
        };
        Self {
            network,
            rpc_url,
            network_passphrase,
        }
    }

    /// Resolve CLI network options over project/user `[network]` defaults.
    pub fn resolve_with_config(
        network: Option<String>,
        rpc_url: Option<String>,
        network_passphrase: Option<String>,
        config: Option<&soroban_forge_core::config::NetworkConfig>,
    ) -> Self {
        let cfg = config.cloned().unwrap_or_default();
        let network = match (network.as_ref(), rpc_url.as_ref()) {
            (Some(name), _) => Some(name.clone()),
            (None, None) => Some(cfg.name.unwrap_or_else(|| DEFAULT_NETWORK.to_string())),
            (None, Some(_)) => cfg.name,
        };
        Self {
            network,
            rpc_url: rpc_url.or(cfg.rpc_url),
            network_passphrase: network_passphrase.or(cfg.passphrase),
        }
    }

    /// What to show in the report as "the network we deployed to".
    pub fn label(&self) -> String {
        self.network
            .clone()
            .or_else(|| self.rpc_url.clone())
            .unwrap_or_else(|| DEFAULT_NETWORK.to_string())
    }

    /// Whether this target is testnet (where friendbot is available).
    pub fn is_testnet(&self) -> bool {
        if let Some(passphrase) = &self.network_passphrase {
            if passphrase.contains("Public Global Stellar Network") {
                return false;
            }
        }
        if let Some(network) = &self.network {
            network == "testnet"
        } else if let Some(rpc_url) = &self.rpc_url {
            rpc_url.contains("testnet")
        } else {
            true // default network is testnet
        }
    }

    /// Whether this target is Stellar public network, which warrants an
    /// explicit confirmation before spending real funds.
    pub fn is_mainnet(&self) -> bool {
        self.network
            .as_deref()
            .is_some_and(|network| matches!(network, "mainnet" | "public"))
            || self
                .network_passphrase
                .as_deref()
                .is_some_and(|passphrase| passphrase.contains("Public Global Stellar Network"))
    }

    /// The corresponding `stellar` CLI arguments.
    pub fn cli_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(network) = &self.network {
            args.push("--network".to_string());
            args.push(network.clone());
        }
        if let Some(rpc_url) = &self.rpc_url {
            args.push("--rpc-url".to_string());
            args.push(rpc_url.clone());
        }
        if let Some(passphrase) = &self.network_passphrase {
            args.push("--network-passphrase".to_string());
            args.push(passphrase.clone());
        }
        args
    }
}

pub fn build_stellar_build_args(extra_args: &[String]) -> Vec<String> {
    let mut args = vec!["contract".to_string(), "build".to_string()];
    args.extend_from_slice(extra_args);
    args
}

fn path_str(path: &Path) -> Result<&str> {
    path.to_str().ok_or_else(|| ForgeError::Other(format!(
        "wasm path {} is not valid UTF-8; normal deployment passes native OS paths directly to stellar-cli, \
         but --dry-run needs printable arguments. Pass --wasm from an ASCII-only path for --dry-run",
        path.display()
    )))
}

/// Build the contract in `dir` with the official `stellar contract build`.
/// Never reimplemented locally.
///
/// Thin system-touching wrapper; not unit-tested.
fn stream_command(command: &mut std::process::Command) -> std::io::Result<Output> {
    let mut child = command.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let mut stdout = child.stdout.take().expect("stdout was piped");
    let mut stderr = child.stderr.take().expect("stderr was piped");
    let stdout_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let count = stdout.read(&mut chunk)?;
            if count == 0 {
                break;
            }
            std::io::stdout().write_all(&chunk[..count])?;
            std::io::stdout().flush()?;
            bytes.extend_from_slice(&chunk[..count]);
        }
        Ok::<_, std::io::Error>(bytes)
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let count = stderr.read(&mut chunk)?;
            if count == 0 {
                break;
            }
            std::io::stderr().write_all(&chunk[..count])?;
            std::io::stderr().flush()?;
            bytes.extend_from_slice(&chunk[..count]);
        }
        Ok::<_, std::io::Error>(bytes)
    });
    let status = child.wait()?;
    let stdout = stdout_reader.join().expect("stdout reader did not panic")?;
    let stderr = stderr_reader.join().expect("stderr reader did not panic")?;
    Ok(Output { status, stdout, stderr })
}

fn run_stellar_build(dir: &Path, verbose: bool) -> Result<()> {
    let mut command = std::process::Command::new("stellar");
    command.args(["contract", "build"]).current_dir(dir);
    let result = if verbose {
        stream_command(&mut command)
    } else {
        command.output()
    };

    match result {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            Err(ForgeError::Other(format!(
                "stellar contract build failed:\n{stderr}"
            )))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(ForgeError::ToolMissing("stellar-cli".into()))
        }
        Err(e) => Err(ForgeError::io("running stellar contract build")(e)),
    }
}

/// Resolve the wasm to deploy: `wasm_override` when given, otherwise the
/// release build of the cargo project in `dir` — building it first with
/// `stellar contract build` if it is not there yet.
pub fn build_if_needed(dir: &Path, wasm_override: Option<&Path>) -> Result<PathBuf> {
    build_if_needed_with_output(dir, wasm_override, false)
}

fn build_if_needed_with_output(
    dir: &Path,
    wasm_override: Option<&Path>,
    verbose: bool,
) -> Result<PathBuf> {
    if let Some(path) = wasm_override {
        return Ok(path.to_path_buf());
    }

    let crate_name = read_crate_name(dir)?;
    let wasm_path = locate_wasm(dir, &crate_name);
    if !wasm_path.is_file() {
        run_stellar_build(dir, verbose)?;
    }
    if !wasm_path.is_file() {
        return Err(ForgeError::Other(format!(
            "stellar contract build did not produce {} — check the build output above",
            wasm_path.display()
        )));
    }
    Ok(wasm_path)
}

/// Parse a `NAME=VALUE` constructor argument string into `(name, value)`.
///
/// Returns `Err` when the string does not contain `=` or the name part is
/// empty, so a bare `=value` or a plain word fails fast with a clear message.
pub fn parse_constructor_arg(raw: &str) -> Result<(String, String)> {
    let eq = raw.find('=').ok_or_else(|| {
        ForgeError::InvalidArgument(format!(
            "constructor argument `{raw}` must be in NAME=VALUE form (e.g. admin=G...)"
        ))
    })?;
    let name = raw[..eq].trim();
    if name.is_empty() {
        return Err(ForgeError::InvalidArgument(format!(
            "constructor argument `{raw}` has an empty name; expected NAME=VALUE"
        )));
    }
    Ok((name.to_string(), raw[eq + 1..].to_string()))
}

pub fn parse_constructor_args(raw: &[String]) -> Result<Vec<(String, String)>> {
    raw.iter().map(|s| parse_constructor_arg(s)).collect()
}

/// Assemble the full `stellar contract deploy` argument list, optionally
/// including constructor arguments forwarded after `--`.
pub fn build_deploy_args(wasm: &Path, source: &str, network: &NetworkArgs) -> Result<Vec<String>> {
    build_deploy_args_with_alias(wasm, source, network, None)
}

/// Assemble the deploy command, optionally registering a stellar-cli alias
/// for the resulting contract ID.
pub fn build_deploy_args_with_alias(
    wasm: &Path,
    source: &str,
    network: &NetworkArgs,
    alias: Option<&str>,
) -> Result<Vec<String>> {
    let wasm_str = path_str(wasm)?.to_string();
    let mut args = vec![
        "contract".to_string(),
        "deploy".to_string(),
        "--wasm".to_string(),
        wasm_str,
        "--source".to_string(),
        source.to_string(),
        "--output".to_string(),
        "json".to_string(),
    ];
    if let Some(alias) = alias {
        args.push("--alias".to_string());
        args.push(alias.to_string());
    }
    args.extend(network.cli_args());
    Ok(args)
}

/// Assemble `stellar contract upgrade` arguments for an existing contract.
pub fn build_upgrade_args(
    contract_id: &str,
    wasm: &Path,
    source: &str,
    network: &NetworkArgs,
) -> Result<Vec<String>> {
    let wasm_str = path_str(wasm)?.to_string();
    let mut args = vec![
        "contract".to_string(),
        "upgrade".to_string(),
        "--contract-id".to_string(),
        contract_id.to_string(),
        "--wasm".to_string(),
        wasm_str,
        "--source".to_string(),
        source.to_string(),
    ];
    args.extend(network.cli_args());
    Ok(args)
}

/// Like [`build_deploy_args`] but appends constructor arguments after `--`
/// when any are given.
pub fn build_deploy_args_with_constructor(
    wasm: &Path,
    source: &str,
    network: &NetworkArgs,
    constructor_args: &[(String, String)],
) -> Result<Vec<String>> {
    let mut args = build_deploy_args(wasm, source, network)?;
    if !constructor_args.is_empty() {
        args.push("--".to_string());
        for (name, value) in constructor_args {
            args.push(format!("--{name}"));
            args.push(value.clone());
        }
    }
    Ok(args)
}

/// Deploy `wasm` with `stellar contract deploy` and return the resulting
/// contract ID. Never reimplemented locally.
///
/// Thin system-touching wrapper; not unit-tested.
fn run_stellar_deploy(
    wasm: &Path,
    source: &str,
    network: &NetworkArgs,
    constructor_args: &[(String, String)],
    timeout: Option<Duration>,
    verbose: bool,
) -> Result<String> {
    log::debug!("deploying {}", wasm.display());

    let args = build_deploy_args_with_constructor(wasm, source, network, constructor_args)?;
    let mut command = std::process::Command::new("stellar");
    command.args(&args);
    let result = if verbose {
        stream_command(&mut command)
    } else {
        soroban_forge_core::timeout::output_with_timeout(&mut command, timeout)
    };

    match result {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            extract_contract_id(&stdout).ok_or_else(|| {
                ForgeError::Other(format!(
                    "stellar contract deploy succeeded but no contract ID was found in its output:\n{stdout}"
                ))
            })
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            Err(ForgeError::Other(format!(
                "stellar contract deploy failed:\n{stderr}"
            )))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(ForgeError::ToolMissing("stellar-cli".into()))
        }
        Err(e) => Err(ForgeError::io("running stellar contract deploy")(e)),
    }
}

/// Pull the contract ID out of `stellar contract deploy` output. JSON output
/// is preferred when supported by stellar-cli; the text fallback accepts only
/// checksum-valid StrKey contract IDs, never merely a C-prefixed lookalike.
pub fn extract_contract_id(stdout: &str) -> Option<String> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(stdout) {
        let id = value
            .get("contract_id")
            .and_then(serde_json::Value::as_str)
            .or_else(|| value.as_str());
        if let Some(id) = id {
            if stellar_strkey::Contract::from_string(id).is_ok() {
                return Some(id.to_string());
            }
        }
    }
    stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .rfind(|l| stellar_strkey::Contract::from_string(l).is_ok())
        .map(str::to_string)
}

/// Build (if needed) and deploy the contract in `dir`, returning the new
/// contract ID.
pub fn deploy(
    dir: &Path,
    wasm_override: Option<&Path>,
    source: &str,
    network: &NetworkArgs,
    constructor_args: &[(String, String)],
    timeout: Option<Duration>,
) -> Result<String> {
    deploy_with_output(dir, wasm_override, source, network, constructor_args, timeout, false)
}

fn deploy_with_output(
    dir: &Path,
    wasm_override: Option<&Path>,
    source: &str,
    network: &NetworkArgs,
    constructor_args: &[(String, String)],
    timeout: Option<Duration>,
    verbose: bool,
) -> Result<String> {
    let wasm_path = build_if_needed_with_output(dir, wasm_override, verbose)?;
    run_stellar_deploy(&wasm_path, source, network, constructor_args, timeout, verbose)
}

fn run_stellar_upgrade(
    contract_id: &str,
    wasm: &Path,
    source: &str,
    network: &NetworkArgs,
    timeout: Option<Duration>,
    verbose: bool,
) -> Result<()> {
    let args = build_upgrade_args(contract_id, wasm, source, network)?;
    let mut command = std::process::Command::new("stellar");
    command.args(&args);
    let result = if verbose {
        stream_command(&mut command)
    } else {
        soroban_forge_core::timeout::output_with_timeout(&mut command, timeout)
    };
    match result {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(ForgeError::Other(format!(
            "stellar contract upgrade failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(ForgeError::ToolMissing("stellar-cli".into()))
        }
        Err(e) => Err(ForgeError::io("running stellar contract upgrade")(e)),
    }
}

/// Names of arguments that may contain secret material and must be redacted
/// in dry-run output. The values of these flags are replaced with `<redacted>`.
const SECRET_FLAGS: &[&str] = &["--source", "--secret-key", "--private-key"];

/// Return a shell-quoted command string with secret values redacted.
///
/// `program` is the executable name (e.g. `"stellar"`), `args` is the list of
/// arguments that would be passed. The result is suitable for printing to
/// stdout; it never contains actual key material.
pub fn format_dry_run_command(program: &str, args: &[String]) -> String {
    let mut parts: Vec<String> = vec![program.to_string()];
    let mut redact_next = false;
    for arg in args {
        if redact_next {
            parts.push("<redacted>".to_string());
            redact_next = false;
        } else if SECRET_FLAGS.contains(&arg.as_str()) {
            parts.push(shell_quote(arg));
            redact_next = true;
        } else {
            parts.push(shell_quote(arg));
        }
    }
    parts.join(" ")
}

/// Minimal shell-quoting: wrap in single quotes when the value contains
/// characters that would be interpreted by a shell.
fn shell_quote(s: &str) -> String {
    if s.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':')) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

/// Resolve the public key (`G...`) for `source`.
pub fn resolve_source_public_key(source: &str) -> Option<String> {
    let trimmed = source.trim();
    if trimmed.starts_with('G') && trimmed.len() == 56 {
        return Some(trimmed.to_string());
    }

    // Check ~/.config/soroban-forge/identities.json
    if let Some(config_dir) = dirs::config_dir() {
        let path = config_dir.join("soroban-forge").join("identities.json");
        if let Ok(raw) = std::fs::read_to_string(&path) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(pk) = val
                    .get("identities")
                    .and_then(|ids| ids.get(trimmed))
                    .and_then(|id| id.get("public_key"))
                    .and_then(|pk| pk.as_str())
                {
                    if pk.starts_with('G') && pk.len() == 56 {
                        return Some(pk.to_string());
                    }
                }
            }
        }
    }

    // Try `stellar keys address <source>`
    let output = std::process::Command::new("stellar")
        .args(["keys", "address", trimmed])
        .output();
    if let Ok(out) = output {
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines().map(str::trim) {
                if line.starts_with('G') && line.len() == 56 {
                    return Some(line.to_string());
                }
            }
        }
    }

    None
}

/// Read the default identity set via `soroban-forge identity use <name>`.
pub fn default_identity_from_store() -> Option<String> {
    let config_dir = dirs::config_dir()?;
    let path = config_dir.join("soroban-forge").join("identities.json");
    let raw = std::fs::read_to_string(path).ok()?;
    let val = serde_json::from_str::<serde_json::Value>(&raw).ok()?;
    val.get("default")?.as_str().map(String::from)
}

/// Resolve the source identity to use: CLI override -> config identity.default -> identities.json default.
pub fn resolve_source_identity(
    cli_source: Option<&str>,
    config: Option<&soroban_forge_core::config::ForgeConfig>,
) -> Option<String> {
    if let Some(s) = cli_source {
        return Some(s.to_string());
    }
    if let Some(cfg_default) = config.and_then(|c| c.identity.default.as_ref()) {
        return Some(cfg_default.clone());
    }
    default_identity_from_store()
}

/// Resolve the secret for `source` when the active backend is the OS keychain.
///
/// With backend = "file" the caller keeps passing `--source <name>` and
/// stellar looks the identity up itself; with backend = "keychain" the secret
/// is fetched from the OS keychain and handed to stellar as `--source <S...>`
/// (issue #616, "deploy/invoke key hand-off").
pub fn resolve_source_secret(source: &str, ctx: &soroban_forge_core::ForgeContext) -> Option<String> {
    let backend = soroban_forge_identity::active_backend(ctx).ok()?;
    if backend != soroban_forge_identity::SecretBackend::Keychain {
        return None;
    }
    soroban_forge_identity::resolve_secret_for_handoff(source, ctx).ok()
}

/// Return the closest locally managed identity name, if the source looks like
/// a near miss. This is advisory only: identities managed by stellar-cli must
/// still be allowed through to the underlying command.
fn source_identity_suggestion(source: &str) -> Option<String> {
    let source = source.trim();
    if source.starts_with('G') || source.starts_with('S') {
        return None;
    }
    let config_dir = dirs::config_dir()?;
    let path = config_dir.join("soroban-forge").join("identities.json");
    let raw = std::fs::read_to_string(path).ok()?;
    let val = serde_json::from_str::<serde_json::Value>(&raw).ok()?;
    let identities = val.get("identities")?.as_object()?;
    if identities.contains_key(source) {
        return None;
    }
    closest_identity_name(source, identities.keys().map(String::as_str))
}

fn closest_identity_name<'a>(source: &str, names: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let mut closest: Option<(usize, &str)> = None;
    for name in names {
        let distance = levenshtein_distance(source, name);
        if closest.map(|(best, _)| distance < best).unwrap_or(true) {
            closest = Some((distance, name));
        }
    }
    let threshold = (source.chars().count().max(3) / 3).max(1);
    closest
        .filter(|(distance, _)| *distance <= threshold)
        .map(|(_, name)| name.to_string())
}

fn levenshtein_distance(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    for (left_index, left_char) in left.chars().enumerate() {
        let mut current = vec![left_index + 1];
        for (right_index, right_char) in right.iter().enumerate() {
            let replacement = previous[right_index] + usize::from(left_char != *right_char);
            current.push((previous[right_index + 1] + 1).min(current[right_index] + 1).min(replacement));
        }
        previous = current;
    }
    previous[right.len()]
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

/// Check if `public_key` is already funded on testnet Horizon.
pub fn is_account_funded(public_key: &str, timeout: Option<Duration>) -> Result<bool> {
    let url = format!("https://horizon-testnet.stellar.org/accounts/{public_key}");
    match http_get(&url, timeout) {
        Ok(_) => Ok(true),
        Err(ureq::Error::Status(404, _)) => Ok(false),
        Err(e) => {
            log::warn!("could not query account funding status from Horizon: {e}");
            Err(ForgeError::Other(format!("failed to check account funding status: {e}")))
        }
    }
}

/// Fund `public_key` on testnet via friendbot.
pub fn fund_via_friendbot(
    public_key: &str,
    network_passphrase: Option<&str>,
    timeout: Option<Duration>,
) -> Result<String> {
    if let Some(passphrase) = network_passphrase {
        if passphrase.contains("Public Global Stellar Network") {
            return Err(ForgeError::InvalidArgument(
                "friendbot funding is only available on testnet, not mainnet".into(),
            ));
        }
    }

    let url = format!("https://friendbot.stellar.org/?addr={public_key}");
    log::debug!("requesting friendbot funding for {public_key}: {url}");
    let response = http_get(&url, timeout).map_err(|e| {
        ForgeError::Other(format!(
            "friendbot request failed: {e}
               hint: check your network connection, or the account may already be funded"
        ))
    })?;

    let body = response
        .into_string()
        .map_err(|e| ForgeError::io("reading friendbot response")(e))?;

    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
        if let Some(balance) = v
            .get("balances")
            .and_then(|b| b.as_array())
            .and_then(|arr| {
                arr.iter().find_map(|item| {
                    if item.get("asset_type").and_then(|t| t.as_str()) == Some("native") {
                        item.get("balance").and_then(|b| b.as_str())
                    } else {
                        None
                    }
                })
            })
        {
            return Ok(balance.to_string());
        }
    }

    Ok("10000".to_string())
}

/// Ensure `source` is funded on testnet, prompting or using `--fund`.
pub fn ensure_source_funded(
    source: &str,
    network: &NetworkArgs,
    auto_fund: bool,
    ctx: &ForgeContext,
) -> Result<()> {
    if !network.is_testnet() || ctx.offline {
        return Ok(());
    }

    let Some(pubkey) = resolve_source_public_key(source) else {
        return Ok(());
    };

    use std::io::IsTerminal;
    let is_interactive = !ctx.quiet && !ctx.json && std::io::stdin().is_terminal() && std::io::stdout().is_terminal();

    ensure_source_funded_with(
        source,
        &pubkey,
        network,
        auto_fund,
        is_interactive,
        |pk| is_account_funded(pk, ctx.timeout()),
        |pk| fund_via_friendbot(pk, network.network_passphrase.as_deref(), ctx.timeout()),
    )
}

/// Inner logic for detecting unfunded testnet source and funding it or prompting.
pub fn ensure_source_funded_with<F, G>(
    source: &str,
    pubkey: &str,
    network: &NetworkArgs,
    auto_fund: bool,
    is_interactive: bool,
    mut is_funded_fn: F,
    mut fund_fn: G,
) -> Result<()>
where
    F: FnMut(&str) -> Result<bool>,
    G: FnMut(&str) -> Result<String>,
{
    if !network.is_testnet() {
        return Ok(());
    }

    let funded = is_funded_fn(pubkey)?;
    if funded {
        return Ok(());
    }

    if auto_fund {
        let balance = fund_fn(pubkey)?;
        log::info!("funded `{source}` ({pubkey}) via friendbot: {balance} XLM");
        Ok(())
    } else if is_interactive {
        if confirm(&format!("source account `{source}` ({pubkey}) is not funded on testnet. Fund it via friendbot?")) {
            let balance = fund_fn(pubkey)?;
            log::info!("funded `{source}` ({pubkey}) via friendbot: {balance} XLM");
            Ok(())
        } else {
            Err(ForgeError::InvalidArgument(format!(
                "aborted: source account `{source}` is unfunded on testnet"
            )))
        }
    } else {
        Err(ForgeError::InvalidArgument(format!(
            "source account `{source}` ({pubkey}) is not funded on testnet; pass --fund to fund it via friendbot before deploying"
        )))
    }
}

fn confirm(prompt: &str) -> bool {
    use std::io::Write;
    print!("{prompt} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// Default file name for the deployments record, written next to `forge.toml`
/// (or the project root when no `forge.toml` exists).
pub const DEPLOYMENTS_FILE: &str = "deployments.json";

/// A single recorded deployment.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DeploymentRecord {
    /// The deployed contract ID (strkey `C…`).
    pub contract_id: String,
    /// Network name or RPC URL used during deployment.
    pub network: String,
    /// ISO-8601 UTC timestamp of the deployment.
    pub deployed_at: String,
}

/// Deployments file: a map from `"<network>/<crate_name>"` to a list of
/// `DeploymentRecord` entries (newest last).
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeploymentsFile {
    #[serde(default)]
    pub deployments: std::collections::BTreeMap<String, Vec<DeploymentRecord>>,
}

impl DeploymentsFile {
    /// Load from `path`, returning `Ok(Default)` when the file does not exist.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.is_file() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)
            .map_err(ForgeError::io(format!("reading {}", path.display())))?;
        serde_json::from_str(&raw).map_err(|e| {
            ForgeError::InvalidArgument(format!(
                "could not parse deployments file {}: {e}",
                path.display()
            ))
        })
    }

    /// Append `record` under key `<network>/<name>` and save.
    pub fn record_and_save(
        path: &Path,
        name: &str,
        network: &str,
        contract_id: &str,
    ) -> Result<()> {
        let mut file = Self::load(path)?;
        let key = format!("{network}/{name}");
        let record = DeploymentRecord {
            contract_id: contract_id.to_string(),
            network: network.to_string(),
            deployed_at: utc_now_iso8601(),
        };
        file.deployments.entry(key).or_default().push(record);
        let json = serde_json::to_string_pretty(&file)
            .map_err(|e| ForgeError::Other(format!("serialising deployments: {e}")))?;
        std::fs::write(path, json + "\n")
            .map_err(ForgeError::io(format!("writing {}", path.display())))
    }

    /// Return the most recently recorded contract ID for `<network>/<name>`,
    /// if one exists.
    pub fn latest_contract_id(&self, name: &str, network: &str) -> Option<&str> {
        let key = format!("{network}/{name}");
        self.deployments
            .get(&key)
            .and_then(|records| records.last())
            .map(|r| r.contract_id.as_str())
    }
}

/// Current UTC time formatted as an ISO-8601 string without sub-second precision.
fn utc_now_iso8601() -> String {
    // Use std only; no chrono dependency.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Decompose Unix timestamp into calendar fields (Gregorian, UTC).
    let (y, mo, d, h, mi, s) = unix_secs_to_ymdhms(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

fn unix_secs_to_ymdhms(mut secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    let s = (secs % 60) as u32;
    secs /= 60;
    let mi = (secs % 60) as u32;
    secs /= 60;
    let h = (secs % 24) as u32;
    secs /= 24;
    // Days since 1970-01-01
    let (y, mo, d) = days_to_ymd(secs as u32);
    (y, mo, d, h, mi, s)
}

fn days_to_ymd(days: u32) -> (u32, u32, u32) {
    // Algorithm: civil_from_days by Howard Hinnant (public domain).
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    (y, mo, d)
}

/// Locate the deployments file relative to a project directory.
///
/// We write it to `dir/deployments.json` (same level as `forge.toml`).
pub fn deployments_path(dir: &Path) -> PathBuf {
    dir.join(DEPLOYMENTS_FILE)
}

/// Look up the most recent contract ID for `name` on `network` from the
/// deployments file in `dir`.  Returns `None` if no record exists.
pub fn lookup_recorded_contract_id(dir: &Path, name: &str, network: &str) -> Option<String> {
    let path = deployments_path(dir);
    let file = DeploymentsFile::load(&path).ok()?;
    file.latest_contract_id(name, network)
        .map(str::to_string)
}

/// The `deploy` subcommand.
pub struct DeployPlugin;

impl ForgePlugin for DeployPlugin {
    fn name(&self) -> &'static str {
        "deploy"
    }

    fn command(&self) -> Command {
        Command::new("deploy")
            .about("Build (if needed) and deploy the contract, printing its contract ID")
            .arg(
                Arg::new("path")
                    .long("path")
                    .help("Contract project directory [default: current directory]"),
            )
            .arg(
                Arg::new("wasm")
                    .long("wasm")
                    .help("Path to a pre-built .wasm to deploy [default: build then use target/wasm32v1-none/release/<crate>.wasm]"),
            )
            .arg(
                Arg::new("source")
                    .long("source")
                    .short('s')
                    .value_name("IDENTITY")
                    .help("Source account/identity that funds and signs the deployment [default: config identity.default]"),
            )
            .arg(
                Arg::new("network")
                    .long("network")
                    .short('n')
                    .action(clap::ArgAction::Append)
                    .help("Configured network to deploy to [default: testnet]"),
            )
            .arg(
                Arg::new("rpc-url")
                    .long("rpc-url")
                    .action(clap::ArgAction::Append)
                    .help("RPC endpoint to use instead of a configured network"),
            )
            .arg(
                Arg::new("network-passphrase")
                    .long("network-passphrase")
                    .action(clap::ArgAction::Append)
                    .help("Network passphrase for --rpc-url"),
            )
            .arg(
                Arg::new("dry-run")
                    .long("dry-run")
                    .action(ArgAction::SetTrue)
                    .help("Print the stellar command that would be run without submitting anything"),
            )
            .arg(
                Arg::new("fund")
                    .long("fund")
                    .action(ArgAction::SetTrue)
                    .help("Automatically fund an unfunded testnet source account via friendbot before deploying"),
            )
            .arg(
                Arg::new("upgrade")
                    .long("upgrade")
                    .value_name("CONTRACT_ID")
                    .help("Upgrade this existing contract instead of creating a new one"),
            )
            .arg(
                Arg::new("arg")
                    .long("arg")
                    .value_name("NAME=VALUE")
                    .action(clap::ArgAction::Append)
                    .help("Pass a constructor argument as NAME=VALUE (repeatable)"),
            )
            .arg(
                Arg::new("no-record")
                    .long("no-record")
                    .action(clap::ArgAction::SetTrue)
                    .help("Skip appending this deployment to deployments.json"),
            )
            .arg(
                Arg::new("build-arg")
                    .long("build-arg")
                    .value_name("ARG")
                    .action(clap::ArgAction::Append)
                    .allow_hyphen_values(true)
                    .help("Forward one argument to stellar contract build (repeatable, e.g. --build-arg=--features)"),
            )
            .arg(
                Arg::new("alias")
                    .long("alias")
                    .value_name("NAME")
                    .help("Register the deployed contract under this stellar-cli alias"),
            )
    }

    fn run(&self, matches: &ArgMatches, ctx: &ForgeContext) -> Result<()> {
        let dry_run = matches.get_flag("dry-run");
        let auto_fund = matches.get_flag("fund");
        let no_record = matches.get_flag("no-record");

        // --dry-run does not submit anything but does need to resolve the wasm
        // path to show the full command; it is therefore allowed in offline mode.
        if ctx.offline && !dry_run {
            return Err(ForgeError::InvalidArgument(
                "deploy is unavailable in offline mode because it submits a transaction".into(),
            ));
        }

        let dir = matches
            .get_one::<String>("path")
            .map(|p| ctx.cwd.join(p))
            .unwrap_or_else(|| ctx.cwd.clone());
        let wasm_override = matches.get_one::<String>("wasm").map(|p| ctx.cwd.join(p));
        let source = resolve_source_identity(
            matches.get_one::<String>("source").map(String::as_str),
            ctx.config.as_ref(),
        )
        .ok_or_else(|| {
            ForgeError::InvalidArgument(
                "missing source identity: pass --source or set a default identity (via `soroban-forge identity use <name>` or [identity].default in config)"
                    .into(),
            )
        })?;
        let upgrade_contract_id = matches.get_one::<String>("upgrade");
        let constructor_args = parse_constructor_args(
            &matches
                .get_many::<String>("arg")
                .map(|vals| vals.cloned().collect::<Vec<_>>())
                .unwrap_or_default(),
        )?;

        if let Some(suggestion) = source_identity_suggestion(&source) {
            if !ctx.quiet && !ctx.json {
                eprintln!(
                    "note: `{source}` is not a local soroban-forge identity; did you mean `{suggestion}`? \
                     continuing so stellar-cli-managed identities still work."
                );
            }
        }

        let network = NetworkArgs::resolve_with_config(
            matches.get_one::<String>("network").cloned(),
            matches.get_one::<String>("rpc-url").cloned(),
            matches.get_one::<String>("network-passphrase").cloned(),
            ctx.config.as_ref().map(|config| &config.network),
        );

        if !dry_run && !ctx.offline && network.is_testnet() {
            ensure_source_funded(&source, &network, auto_fund, ctx)?;
        }

        if dry_run {
            let wasm_path = build_if_needed(&dir, wasm_override.as_deref())?;
            let args = match upgrade_contract_id {
                Some(contract_id) => build_upgrade_args(contract_id, &wasm_path, &source_for_stellar, &network)?,
                None => build_deploy_args_with_constructor(&wasm_path, &source_for_stellar, &network, &constructor_args)?,
            };
            let command_line = format_dry_run_command("stellar", &args);
            if ctx.json {
                let report = serde_json::json!({ "commands": [command_line] });
                println!("{}", serde_json::to_string_pretty(&report).unwrap());
            } else {
                println!("{command_line}");
            }
            return Ok(());
        }

        if let Some(contract_id) = upgrade_contract_id {
            let wasm_path = build_if_needed_with_output(
                &dir,
                wasm_override.as_deref(),
                ctx.verbose > 0,
            )?;
            run_stellar_upgrade(
                contract_id,
                &wasm_path,
                &source,
                &network,
                ctx.timeout(),
                ctx.verbose > 0,
            )?;
            if ctx.json {
                let report = serde_json::json!({
                    "contract_id": contract_id,
                    "network": network.label(),
                    "upgraded": true,
                });
                println!("{}", serde_json::to_string_pretty(&report).unwrap());
            } else if !ctx.quiet {
                println!("upgraded contract {contract_id} on {}", network.label());
            }
            return Ok(());
        }

        let contract_id = deploy_with_output(
            &dir,
            wasm_override.as_deref(),
            &source_for_stellar,
            &network,
            &constructor_args,
            ctx.timeout(),
            ctx.verbose > 0,
        )?;

        // Record the deployment unless --no-record was passed.
        if !no_record {
            let crate_name = read_crate_name(&dir).unwrap_or_else(|_| "unknown".to_string());
            let deployments_file = deployments_path(&dir);
            if let Err(e) =
                DeploymentsFile::record_and_save(&deployments_file, &crate_name, &network.label(), &contract_id)
            {
                log::warn!("could not write deployments file: {e}");
            }
        }

        if ctx.json {
            let report = serde_json::json!({
                "contract_id": contract_id,
                "network": network.label(),
            });
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
        } else if !ctx.quiet {
            println!("deployed to {}", network.label());
            println!("contract ID: {contract_id}");
        } else {
            println!("{contract_id}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract_id(seed: u8) -> String {
        stellar_strkey::Contract([seed; 32]).to_string()
    }

    #[test]
    fn locates_wasm_by_crate_name() {
        assert_eq!(
            locate_wasm(Path::new("/proj"), "my_token"),
            PathBuf::from("/proj/target/wasm32v1-none/release/my_token.wasm")
        );
    }

    #[test]
    fn reads_and_normalizes_the_crate_name() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"my-token\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();

        assert_eq!(read_crate_name(tmp.path()).unwrap(), "my_token");
    }

    #[test]
    fn errors_outside_a_cargo_project() {
        let tmp = tempfile::tempdir().unwrap();
        let err = read_crate_name(tmp.path()).unwrap_err();
        assert!(err.to_string().contains("not a cargo project"), "{err}");
    }

    #[test]
    fn wasm_override_skips_crate_name_lookup() {
        let tmp = tempfile::tempdir().unwrap();
        let custom = tmp.path().join("custom.wasm");
        std::fs::write(&custom, b"\0asm").unwrap();

        // No Cargo.toml here at all — build_if_needed must not need one when
        // an explicit --wasm is given.
        assert_eq!(build_if_needed(tmp.path(), Some(&custom)).unwrap(), custom);
    }

    #[test]
    fn defaults_to_testnet() {
        let network = NetworkArgs::resolve(None, None, None);
        assert_eq!(network.label(), "testnet");
        assert_eq!(network.cli_args(), vec!["--network", "testnet"]);
    }

    #[test]
    fn project_or_user_network_defaults_are_used_but_cli_wins() {
        let config = soroban_forge_core::config::NetworkConfig {
            name: Some("mainnet".into()),
            rpc_url: Some("https://configured.example".into()),
            passphrase: Some("configured passphrase".into()),
        };
        let configured = NetworkArgs::resolve_with_config(None, None, None, Some(&config));
        assert_eq!(configured.network.as_deref(), Some("mainnet"));
        assert_eq!(configured.rpc_url.as_deref(), Some("https://configured.example"));
        assert_eq!(configured.network_passphrase.as_deref(), Some("configured passphrase"));

        let cli = NetworkArgs::resolve_with_config(
            Some("testnet".into()),
            Some("https://cli.example".into()),
            Some("cli passphrase".into()),
            Some(&config),
        );
        assert_eq!(cli.network.as_deref(), Some("testnet"));
        assert_eq!(cli.rpc_url.as_deref(), Some("https://cli.example"));
        assert_eq!(cli.network_passphrase.as_deref(), Some("cli passphrase"));
    }

    #[test]
    fn an_explicit_network_is_passed_through() {
        let network = NetworkArgs::resolve(Some("mainnet".into()), None, None);
        assert_eq!(network.cli_args(), vec!["--network", "mainnet"]);
    }

    #[test]
    fn an_rpc_url_replaces_the_default_network() {
        let network = NetworkArgs::resolve(
            None,
            Some("http://localhost:8000/soroban/rpc".into()),
            Some("Standalone Network ; February 2017".into()),
        );
        assert_eq!(network.network, None);
        assert_eq!(
            network.cli_args(),
            vec![
                "--rpc-url",
                "http://localhost:8000/soroban/rpc",
                "--network-passphrase",
                "Standalone Network ; February 2017",
            ]
        );
    }

    #[test]
    fn extracts_only_a_checksum_valid_contract_id() {
        let incidental = format!("C{}", "A".repeat(55));
        let deployed = contract_id(7);
        let stdout = format!("ℹ️ deploying...\n{incidental}\nsuccess\n{deployed}\n");
        assert_eq!(extract_contract_id(&stdout).as_deref(), Some(deployed.as_str()));
    }

    #[test]
    fn extracts_contract_id_from_structured_deploy_output() {
        let deployed = contract_id(8);
        let stdout = format!(r#"{{"contract_id":"{deployed}"}}"#);
        assert_eq!(extract_contract_id(&stdout).as_deref(), Some(deployed.as_str()));
    }

    #[test]
    fn no_contract_id_found_returns_none() {
        assert_eq!(extract_contract_id("deploy failed\n"), None);
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_dry_run_path_explains_the_workaround() {
        use std::os::unix::ffi::OsStringExt;

        let path = PathBuf::from(std::ffi::OsString::from_vec(vec![b'/', 0xFF]));
        let err = path_str(&path).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("not valid UTF-8"), "{message}");
        assert!(message.contains("ASCII-only path"), "{message}");
    }

    #[test]
    fn plugin_name_matches_its_command() {
        let plugin = DeployPlugin;
        assert_eq!(plugin.name(), plugin.command().get_name());
    }

    #[test]
    fn help_documents_source_and_network() {
        let help = DeployPlugin.command().render_long_help().to_string();
        assert!(help.contains("--source"), "{help}");
        assert!(help.contains("--network"), "{help}");
        assert!(help.contains("IDENTITY"), "{help}");
    }

    #[test]
    fn help_documents_dry_run() {
        let help = DeployPlugin.command().render_long_help().to_string();
        assert!(help.contains("--dry-run"), "{help}");
    }

    #[test]
    fn help_documents_upgrade_mode() {
        let help = DeployPlugin.command().render_long_help().to_string();
        assert!(help.contains("--upgrade"), "{help}");
    }

    #[test]
    fn build_deploy_args_assembles_full_command() {
        let tmp = tempfile::tempdir().unwrap();
        let wasm = tmp.path().join("my_contract.wasm");
        std::fs::write(&wasm, b"\0asm").unwrap();
        let network = NetworkArgs::resolve(None, None, None);
        let args = build_deploy_args(&wasm, "alice", &network).unwrap();
        assert_eq!(args[0], "contract");
        assert_eq!(args[1], "deploy");
        assert!(args.contains(&"--wasm".to_string()));
        assert!(args.contains(&"--source".to_string()));
        assert!(args.contains(&"alice".to_string()));
        assert!(args.contains(&"--network".to_string()));
        assert!(args.contains(&"testnet".to_string()));
        assert!(args.contains(&"json".to_string()));
    }

    #[test]
    fn build_upgrade_args_assembles_full_command() {
        let tmp = tempfile::tempdir().unwrap();
        let wasm = tmp.path().join("my_contract.wasm");
        std::fs::write(&wasm, b"\0asm").unwrap();
        let network = NetworkArgs::resolve(None, None, None);
        let contract_id = contract_id(9);
        let args = build_upgrade_args(&contract_id, &wasm, "alice", &network).unwrap();

        assert_eq!(args[0], "contract");
        assert_eq!(args[1], "upgrade");
        assert!(args.windows(2).any(|pair| pair[0] == "--contract-id" && pair[1] == contract_id));
        assert!(args.windows(2).any(|pair| pair[0] == "--source" && pair[1] == "alice"));
        assert!(args.windows(2).any(|pair| pair[0] == "--network" && pair[1] == "testnet"));
    }

    #[test]
    fn build_flags_are_forwarded_to_stellar_contract_build() {
        let args = build_stellar_build_args(&[
            "--features".to_string(),
            "experimental".to_string(),
            "--profile".to_string(),
            "release".to_string(),
        ]);
        assert_eq!(
            args.iter().map(String::as_str).collect::<Vec<_>>(),
            ["contract", "build", "--features", "experimental", "--profile", "release"]
        );
    }

    #[test]
    fn deploy_alias_is_forwarded_to_stellar_cli() {
        let tmp = tempfile::tempdir().unwrap();
        let wasm = tmp.path().join("my_contract.wasm");
        std::fs::write(&wasm, b"\0asm").unwrap();
        let network = NetworkArgs::resolve(None, None, None);
        let args = build_deploy_args_with_alias(&wasm, "alice", &network, Some("my_token")).unwrap();
        assert!(args.windows(2).any(|pair| pair[0] == "--alias" && pair[1] == "my_token"));
    }

    #[test]
    fn format_dry_run_redacts_source_value() {
        let args = vec![
            "contract".to_string(),
            "deploy".to_string(),
            "--source".to_string(),
            "SCZANGBA5YELKNYAXSWI2YQNMN7HAIYE".to_string(),
            "--network".to_string(),
            "testnet".to_string(),
        ];
        let cmd = format_dry_run_command("stellar", &args);
        assert!(!cmd.contains("SCZANGBA5YELKNYAXSWI2YQNMN7HAIYE"), "secret must be redacted: {cmd}");
        assert!(cmd.contains("<redacted>"), "placeholder must be present: {cmd}");
        assert!(cmd.contains("--network"), "network must be retained: {cmd}");
        assert!(cmd.contains("testnet"), "network value must be retained: {cmd}");
    }

    #[test]
    fn format_dry_run_includes_all_non_secret_args() {
        let args = vec![
            "contract".to_string(),
            "deploy".to_string(),
            "--wasm".to_string(),
            "target/wasm32v1-none/release/my.wasm".to_string(),
            "--source".to_string(),
            "alice".to_string(),
            "--network".to_string(),
            "testnet".to_string(),
        ];
        let cmd = format_dry_run_command("stellar", &args);
        assert!(cmd.starts_with("stellar"), "{cmd}");
        assert!(cmd.contains("contract"), "{cmd}");
        assert!(cmd.contains("deploy"), "{cmd}");
        assert!(cmd.contains("--wasm"), "{cmd}");
        // --source value "alice" looks like a plain identity name, but it's
        // still treated as a secret and redacted.
        assert!(cmd.contains("<redacted>"), "{cmd}");
    }

    #[test]
    fn help_documents_fund_flag() {
        let help = DeployPlugin.command().render_long_help().to_string();
        assert!(help.contains("--fund"), "{help}");
    }

    #[test]
    fn help_documents_build_args_and_alias() {
        let help = DeployPlugin.command().render_long_help().to_string();
        assert!(help.contains("--build-arg"), "{help}");
        assert!(help.contains("--alias"), "{help}");
    }

    #[test]
    fn network_args_identifies_testnet_correctly() {
        let testnet_default = NetworkArgs::resolve(None, None, None);
        assert!(testnet_default.is_testnet());

        let testnet_explicit = NetworkArgs::resolve(Some("testnet".into()), None, None);
        assert!(testnet_explicit.is_testnet());

        let mainnet = NetworkArgs::resolve(Some("mainnet".into()), None, None);
        assert!(!mainnet.is_testnet());

        let mainnet_passphrase = NetworkArgs::resolve(
            Some("testnet".into()),
            None,
            Some("Public Global Stellar Network ; September 2015".into()),
        );
        assert!(!mainnet_passphrase.is_testnet());
    }

    #[test]
    fn resolve_source_public_key_accepts_direct_pubkey() {
        let pk = "GBRPYHIL2CI3FNQ4BXLFMNDLFJUNPU2HY3ZMFSHONUCEOASW7QC7OX2H";
        assert_eq!(resolve_source_public_key(pk), Some(pk.to_string()));
    }

    #[test]
    fn source_identity_near_miss_suggests_the_closest_local_name() {
        let suggestion = closest_identity_name("deployer", ["deployer-prod", "alice", "deployr"]);
        assert_eq!(suggestion.as_deref(), Some("deployr"));
    }

    #[test]
    fn ensure_source_funded_skips_when_already_funded() {
        let network = NetworkArgs::resolve(Some("testnet".into()), None, None);
        let mut funded_called = false;
        let res = ensure_source_funded_with(
            "alice",
            "GBRPYHIL2CI3FNQ4BXLFMNDLFJUNPU2HY3ZMFSHONUCEOASW7QC7OX2H",
            &network,
            false,
            false,
            |_| Ok(true), // already funded
            |_| {
                funded_called = true;
                Ok("10000".into())
            },
        );
        assert!(res.is_ok());
        assert!(!funded_called);
    }

    #[test]
    fn ensure_source_funded_auto_funds_when_flag_present() {
        let network = NetworkArgs::resolve(Some("testnet".into()), None, None);
        let mut funded_called = false;
        let res = ensure_source_funded_with(
            "alice",
            "GBRPYHIL2CI3FNQ4BXLFMNDLFJUNPU2HY3ZMFSHONUCEOASW7QC7OX2H",
            &network,
            true, // auto_fund = true
            false, // non-interactive
            |_| Ok(false), // unfunded
            |_| {
                funded_called = true;
                Ok("10000".into())
            },
        );
        assert!(res.is_ok());
        assert!(funded_called);
    }

    #[test]
    fn ensure_source_funded_fails_non_interactive_without_fund_flag() {
        let network = NetworkArgs::resolve(Some("testnet".into()), None, None);
        let mut funded_called = false;
        let res = ensure_source_funded_with(
            "alice",
            "GBRPYHIL2CI3FNQ4BXLFMNDLFJUNPU2HY3ZMFSHONUCEOASW7QC7OX2H",
            &network,
            false, // auto_fund = false
            false, // is_interactive = false
            |_| Ok(false), // unfunded
            |_| {
                funded_called = true;
                Ok("10000".into())
            },
        );
        assert!(res.is_err());
        assert!(!funded_called);
        let err = res.unwrap_err().to_string();
        assert!(err.contains("pass --fund"), "expected hint to pass --fund, got: {err}");
    }

    #[test]
    fn ensure_source_funded_never_attempts_funding_on_mainnet() {
        let network = NetworkArgs::resolve(Some("mainnet".into()), None, None);
        let mut checked = false;
        let res = ensure_source_funded_with(
            "alice",
            "GBRPYHIL2CI3FNQ4BXLFMNDLFJUNPU2HY3ZMFSHONUCEOASW7QC7OX2H",
            &network,
            true,
            false,
            |_| {
                checked = true;
                Ok(false)
            },
            |_| Ok("10000".into()),
        );
        assert!(res.is_ok());
        assert!(!checked);
    }

    // -----------------------------------------------------------------------
    // Issue #280 — --arg NAME=VALUE (constructor arguments)
    // -----------------------------------------------------------------------

    #[test]
    fn parse_constructor_arg_splits_on_first_equals() {
        let (name, value) = parse_constructor_arg("admin=GABC").unwrap();
        assert_eq!(name, "admin");
        assert_eq!(value, "GABC");
    }

    #[test]
    fn parse_constructor_arg_allows_equals_in_value() {
        let (name, value) = parse_constructor_arg("token=abc=def").unwrap();
        assert_eq!(name, "token");
        assert_eq!(value, "abc=def");
    }

    #[test]
    fn parse_constructor_arg_rejects_missing_equals() {
        let err = parse_constructor_arg("noequalssign").unwrap_err();
        assert!(err.to_string().contains("NAME=VALUE"), "{err}");
    }

    #[test]
    fn parse_constructor_arg_rejects_empty_name() {
        let err = parse_constructor_arg("=value").unwrap_err();
        assert!(err.to_string().contains("empty name"), "{err}");
    }

    #[test]
    fn build_deploy_args_with_constructor_appends_after_separator() {
        let tmp = tempfile::tempdir().unwrap();
        let wasm = tmp.path().join("contract.wasm");
        std::fs::write(&wasm, b"\0asm").unwrap();
        let network = NetworkArgs::resolve(None, None, None);
        let ctor_args = vec![
            ("admin".to_string(), "GABC".to_string()),
            ("decimals".to_string(), "7".to_string()),
        ];
        let args = build_deploy_args_with_constructor(&wasm, "alice", &network, &ctor_args).unwrap();
        let sep = args.iter().position(|a| a == "--").expect("separator not found");
        assert!(args[sep + 1..].contains(&"--admin".to_string()), "{args:?}");
        assert!(args[sep + 1..].contains(&"GABC".to_string()), "{args:?}");
        assert!(args[sep + 1..].contains(&"--decimals".to_string()), "{args:?}");
        assert!(args[sep + 1..].contains(&"7".to_string()), "{args:?}");
    }

    #[test]
    fn build_deploy_args_with_no_constructor_args_omits_separator() {
        let tmp = tempfile::tempdir().unwrap();
        let wasm = tmp.path().join("contract.wasm");
        std::fs::write(&wasm, b"\0asm").unwrap();
        let network = NetworkArgs::resolve(None, None, None);
        let args = build_deploy_args_with_constructor(&wasm, "alice", &network, &[]).unwrap();
        assert!(!args.contains(&"--".to_string()), "{args:?}");
    }

    #[test]
    fn command_exposes_arg_flag() {
        let matches = DeployPlugin
            .command()
            .try_get_matches_from(vec![
                "deploy",
                "--source",
                "alice",
                "--arg",
                "admin=GABC",
                "--arg",
                "decimals=7",
            ])
            .unwrap();
        let args: Vec<&String> = matches.get_many::<String>("arg").unwrap().collect();
        assert_eq!(args, vec!["admin=GABC", "decimals=7"]);
    }

    #[test]
    fn help_documents_arg_flag() {
        let help = DeployPlugin.command().render_long_help().to_string();
        assert!(help.contains("--arg"), "{help}");
        assert!(help.contains("NAME=VALUE"), "{help}");
    }

    // -----------------------------------------------------------------------
    // Issue #281 — deployments.json
    // -----------------------------------------------------------------------

    #[test]
    fn deployments_file_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("deployments.json");
        DeploymentsFile::record_and_save(&path, "my_token", "testnet", "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").unwrap();
        let file = DeploymentsFile::load(&path).unwrap();
        assert_eq!(
            file.latest_contract_id("my_token", "testnet"),
            Some("CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
        );
    }

    #[test]
    fn deployments_file_records_newest_last() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("deployments.json");
        DeploymentsFile::record_and_save(&path, "my_token", "testnet", "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").unwrap();
        DeploymentsFile::record_and_save(&path, "my_token", "testnet", "CBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB").unwrap();
        let file = DeploymentsFile::load(&path).unwrap();
        assert_eq!(
            file.latest_contract_id("my_token", "testnet"),
            Some("CBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB")
        );
    }

    #[test]
    fn deployments_file_missing_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let file = DeploymentsFile::load(&tmp.path().join("deployments.json")).unwrap();
        assert_eq!(file.latest_contract_id("my_token", "testnet"), None);
    }

    #[test]
    fn deployments_file_different_networks_are_separate() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("deployments.json");
        DeploymentsFile::record_and_save(&path, "my_token", "testnet", "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").unwrap();
        DeploymentsFile::record_and_save(&path, "my_token", "mainnet", "CBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB").unwrap();
        let file = DeploymentsFile::load(&path).unwrap();
        assert_eq!(
            file.latest_contract_id("my_token", "testnet"),
            Some("CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
        );
        assert_eq!(
            file.latest_contract_id("my_token", "mainnet"),
            Some("CBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB")
        );
    }

    #[test]
    fn lookup_recorded_contract_id_returns_latest() {
        let tmp = tempfile::tempdir().unwrap();
        DeploymentsFile::record_and_save(
            &tmp.path().join(DEPLOYMENTS_FILE),
            "my_token",
            "testnet",
            "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        ).unwrap();
        assert_eq!(
            lookup_recorded_contract_id(tmp.path(), "my_token", "testnet"),
            Some("CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string())
        );
    }

    #[test]
    fn help_documents_no_record_flag() {
        let help = DeployPlugin.command().render_long_help().to_string();
        assert!(help.contains("--no-record"), "{help}");
    }

    // #427 — resolve_source_identity picks up config default identity when --source is omitted
    #[test]
    fn test_resolve_source_identity_picks_up_default_from_config() {
        let mut config = soroban_forge_core::config::ForgeConfig::default();
        config.identity.default = Some("alice".into());

        let resolved = resolve_source_identity(None, Some(&config));
        assert_eq!(resolved, Some("alice".to_string()));
    }

    // #427 — resolve_source_identity prioritizes explicit CLI --source over config default
    #[test]
    fn test_resolve_source_identity_cli_override_takes_precedence() {
        let mut config = soroban_forge_core::config::ForgeConfig::default();
        config.identity.default = Some("alice".into());

        let resolved = resolve_source_identity(Some("bob"), Some(&config));
        assert_eq!(resolved, Some("bob".to_string()));
    }

    // #427 — deploy --source flag is optional
    #[test]
    fn test_deploy_command_source_flag_is_optional() {
        let plugin = DeployPlugin;
        let cmd = plugin.command();
        let matches = cmd.try_get_matches_from(["deploy", "--dry-run"]);
        assert!(matches.is_ok(), "deploy must parse without --source");
    }
}
