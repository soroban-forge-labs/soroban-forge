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

use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Arg, ArgMatches, Command};
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

fn path_str(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| ForgeError::Other(format!("path {} is not valid UTF-8", path.display())))
}

/// Build the contract in `dir` with the official `stellar contract build`.
/// Never reimplemented locally.
///
/// Thin system-touching wrapper; not unit-tested.
fn run_stellar_build(dir: &Path) -> Result<()> {
    let result = std::process::Command::new("stellar")
        .args(["contract", "build"])
        .current_dir(dir)
        .output();

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
    if let Some(path) = wasm_override {
        return Ok(path.to_path_buf());
    }

    let crate_name = read_crate_name(dir)?;
    let wasm_path = locate_wasm(dir, &crate_name);
    if !wasm_path.is_file() {
        run_stellar_build(dir)?;
    }
    if !wasm_path.is_file() {
        return Err(ForgeError::Other(format!(
            "stellar contract build did not produce {} — check the build output above",
            wasm_path.display()
        )));
    }
    Ok(wasm_path)
}

/// Assemble the full `stellar contract deploy` argument list.
pub fn build_deploy_args(wasm: &Path, source: &str, network: &NetworkArgs) -> Result<Vec<String>> {
    let wasm_str = path_str(wasm)?.to_string();
    let mut args = vec![
        "contract".to_string(),
        "deploy".to_string(),
        "--wasm".to_string(),
        wasm_str,
        "--source".to_string(),
        source.to_string(),
    ];
    args.extend(network.cli_args());
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
    timeout: Option<Duration>,
) -> Result<String> {
    let args = build_deploy_args(wasm, source, network)?;
    log::debug!("deploying {}", wasm.display());

    let result = soroban_forge_core::timeout::output_with_timeout(
        std::process::Command::new("stellar").args(&args),
        timeout,
    );

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

/// Pull the contract ID out of `stellar contract deploy`'s stdout: the last
/// non-empty line that looks like a strkey contract ID (`C` + 55 base32
/// characters).
pub fn extract_contract_id(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .rfind(|l| l.starts_with('C') && l.chars().count() == 56)
        .map(str::to_string)
}

/// Build (if needed) and deploy the contract in `dir`, returning the new
/// contract ID.
pub fn deploy(
    dir: &Path,
    wasm_override: Option<&Path>,
    source: &str,
    network: &NetworkArgs,
    timeout: Option<Duration>,
) -> Result<String> {
    let wasm_path = build_if_needed(dir, wasm_override)?;
    run_stellar_deploy(&wasm_path, source, network, timeout)
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
                    .required(true)
                    .value_name("IDENTITY")
                    .help("Source account/identity that funds and signs the deployment"),
            )
            .arg(
                Arg::new("network")
                    .long("network")
                    .short('n')
                    .help("Configured network to deploy to [default: testnet]"),
            )
            .arg(
                Arg::new("rpc-url")
                    .long("rpc-url")
                    .help("RPC endpoint to use instead of a configured network"),
            )
            .arg(
                Arg::new("network-passphrase")
                    .long("network-passphrase")
                    .help("Network passphrase for --rpc-url"),
            )
            .arg(
                Arg::new("dry-run")
                    .long("dry-run")
                    .action(clap::ArgAction::SetTrue)
                    .help("Print the stellar command that would be run without submitting anything"),
            )
            .arg(
                Arg::new("fund")
                    .long("fund")
                    .action(clap::ArgAction::SetTrue)
                    .help("Automatically fund an unfunded testnet source account via friendbot before deploying"),
            )
    }

    fn run(&self, matches: &ArgMatches, ctx: &ForgeContext) -> Result<()> {
        let dry_run = matches.get_flag("dry-run");
        let auto_fund = matches.get_flag("fund");

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
        let source = matches
            .get_one::<String>("source")
            .expect("source is required by clap");

        let network = NetworkArgs::resolve(
            matches.get_one::<String>("network").cloned(),
            matches.get_one::<String>("rpc-url").cloned(),
            matches.get_one::<String>("network-passphrase").cloned(),
        );

        if !dry_run && !ctx.offline && network.is_testnet() {
            ensure_source_funded(source, &network, auto_fund, ctx)?;
        }

        if dry_run {
            let wasm_path = build_if_needed(&dir, wasm_override.as_deref())?;
            let args = build_deploy_args(&wasm_path, source, &network)?;
            let command_line = format_dry_run_command("stellar", &args);
            if ctx.json {
                let report = serde_json::json!({ "command": command_line });
                println!("{}", serde_json::to_string_pretty(&report).unwrap());
            } else {
                println!("{command_line}");
            }
            return Ok(());
        }

        let contract_id = deploy(&dir, wasm_override.as_deref(), source, &network, ctx.timeout())?;

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
    fn extracts_the_last_contract_id_line() {
        let stdout = "ℹ️ deploying...\nsuccess\nCAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n";
        assert_eq!(
            extract_contract_id(stdout).as_deref(),
            Some("CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
        );
    }

    #[test]
    fn no_contract_id_found_returns_none() {
        assert_eq!(extract_contract_id("deploy failed\n"), None);
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
}
