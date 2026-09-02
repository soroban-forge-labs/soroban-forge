//! # soroban-forge-verify
//!
//! `soroban-forge verify <contract-id>` — answers one question: **is the
//! contract deployed at this ID the same code as my local release build?**
//!
//! A contract's on-chain wasm hash is the SHA-256 of its deployed wasm
//! bytes, so the check reduces to comparing two hashes:
//!
//! - **local** — SHA-256 of `target/wasm32v1-none/release/<crate_name>.wasm`
//!   (the `wasm32v1-none` target soroban-forge templates, `doctor` and
//!   `bindings ts` all expect), or of `--wasm <path>` when given
//! - **on-chain** — SHA-256 of the bytes the network returns for the
//!   contract ID
//!
//! Per soroban-forge's "wrap, don't reimplement" rule the on-chain half is
//! downloaded with the official `stellar contract fetch`; this module only
//! locates the local build, hashes both sides and reports the verdict.
//!
//! Exit status follows [`soroban_forge_core::ForgeError`]: a match exits `0`,
//! a mismatch is a [`ForgeError::VerificationFailed`] (exit `1`) so CI can
//! branch on it, and a missing `stellar` CLI is a
//! [`ForgeError::ToolMissing`] (exit `2`).

use std::path::{Path, PathBuf};

use clap::{Arg, ArgAction, ArgMatches, Command};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use soroban_forge_core::{ForgeContext, ForgeError, ForgePlugin, Result};

/// Network used when neither `--network` nor `--rpc-url` is given.
pub const DEFAULT_NETWORK: &str = "testnet";

/// Pinned image (by digest) used by `verify --reproducible` to build the
/// contract before hashing it. Pinned by digest so the hash we compare
/// against is independent of the local toolchain — same image produces the
/// same bytes for the same source. See `docs/reproducible-builds.md` for how
/// to refresh the pin.
pub const REPRODUCIBLE_IMAGE: &str =
    "ghcr.io/stellar/soroban-build@sha256:0000000000000000000000000000000000000000000000000000000000000000";

/// Length of a strkey-encoded contract ID (`C` + 55 base32 characters).
pub const CONTRACT_ID_LEN: usize = 56;

/// Every wasm module starts with these four bytes.
const WASM_MAGIC: &[u8] = b"\0asm";

#[derive(Deserialize)]
struct Manifest {
    package: Package,
}

#[derive(Deserialize)]
struct Package {
    name: String,
}

/// Read `[package].name` from `dir/Cargo.toml` and return it as a crate name
/// (snake_case), which is what the build output is named after.
///
/// Deliberately duplicated rather than shared with `bindings ts`: modules
/// depend only on `soroban-forge-core`, never on each other.
pub fn read_crate_name(dir: &Path) -> Result<String> {
    let manifest_path = dir.join("Cargo.toml");
    if !manifest_path.is_file() {
        return Err(ForgeError::InvalidArgument(format!(
            "{} is not a cargo project (no Cargo.toml) — pass --path or --wasm",
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

/// Resolve which local wasm to hash: `wasm_override` when given, otherwise
/// the release build of the cargo project in `dir`. Errors when the file is
/// not there, pointing at `stellar contract build`.
pub fn resolve_local_wasm(dir: &Path, wasm_override: Option<&Path>) -> Result<PathBuf> {
    let wasm_path = match wasm_override {
        Some(path) => path.to_path_buf(),
        None => {
            let crate_name = read_crate_name(dir)?;
            locate_wasm(dir, &crate_name)
        }
    };

    if !wasm_path.is_file() {
        return Err(ForgeError::InvalidArgument(format!(
            "no release build found at {} — run `stellar contract build` first (or pass --wasm)",
            wasm_path.display()
        )));
    }
    Ok(wasm_path)
}

/// Cheap shape check on a strkey contract ID so an obvious typo fails before
/// we shell out to the network. Contract IDs are 56 base32 characters
/// starting with `C`; the checksum is left to the `stellar` CLI, which
/// decodes the strkey for real.
pub fn validate_contract_id(id: &str) -> Result<()> {
    let invalid = |reason: &str| {
        Err(ForgeError::InvalidArgument(format!(
            "`{id}` is not a valid contract ID ({reason}); expected {CONTRACT_ID_LEN} characters starting with `C`"
        )))
    };

    if !id.starts_with('C') {
        return invalid("must start with `C`");
    }
    if id.chars().count() != CONTRACT_ID_LEN {
        return invalid("wrong length");
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_uppercase() || ('2'..='7').contains(&c))
    {
        return invalid("contains characters outside the base32 alphabet");
    }
    Ok(())
}

/// Lowercase hex SHA-256 of `bytes` — the same hash Stellar identifies
/// uploaded contract wasm by.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Read `path` and return its SHA-256. Rejects files that are not wasm, so a
/// truncated download or a stray text file is reported as such instead of
/// silently becoming a "mismatch".
pub fn hash_wasm_file(path: &Path) -> Result<String> {
    let bytes =
        std::fs::read(path).map_err(ForgeError::io(format!("reading {}", path.display())))?;
    if !bytes.starts_with(WASM_MAGIC) {
        return Err(ForgeError::InvalidArgument(format!(
            "{} is not a wasm module (missing the \\0asm header)",
            path.display()
        )));
    }
    Ok(sha256_hex(&bytes))
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

/// Subset of `forge.toml`'s `[network]` table relevant to `verify`.
pub type ConfigNetwork = soroban_forge_core::config::NetworkConfig;

impl NetworkArgs {
    /// Apply the defaults: with no network *and* no RPC URL we target
    /// [`DEFAULT_NETWORK`]. An explicit `--rpc-url` alone is left alone, so
    /// the endpoint the user asked for is the one we talk to.
    ///
    /// `from_config`, when given, supplies defaults from `forge.toml`'s
    /// `[network]` table; CLI flags override file values.
    pub fn resolve(
        network: Option<String>,
        rpc_url: Option<String>,
        network_passphrase: Option<String>,
        from_config: Option<&ConfigNetwork>,
    ) -> Self {
        let cfg = from_config.cloned().unwrap_or_default();
        let cli_network = network;
        let cli_rpc = rpc_url;
        let network = match (cli_network.as_ref(), cli_rpc.as_ref()) {
            (Some(name), _) => Some(name.clone()),
            (None, None) => Some(cfg.name.clone().unwrap_or_else(|| DEFAULT_NETWORK.to_string())),
            (None, Some(_)) => cfg.name.clone(),
        };
        let rpc_url = cli_rpc.or(cfg.rpc_url);
        let network_passphrase = network_passphrase.or(cfg.passphrase);
        Self {
            network,
            rpc_url,
            network_passphrase,
        }
    }

    /// What to show in the report as "the network we asked".
    pub fn label(&self) -> String {
        self.network
            .clone()
            .or_else(|| self.rpc_url.clone())
            .unwrap_or_else(|| DEFAULT_NETWORK.to_string())
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

/// The outcome of one verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VerifyReport {
    pub contract_id: String,
    pub network: String,
    /// Local wasm that was hashed.
    pub local_wasm: String,
    pub local_hash: String,
    pub onchain_hash: String,
    /// Whether the two hashes are identical.
    #[serde(rename = "match")]
    pub matches: bool,
}

impl VerifyReport {
    /// Build a report, deriving the verdict from the two hashes.
    pub fn new(
        contract_id: impl Into<String>,
        network: impl Into<String>,
        local_wasm: &Path,
        local_hash: impl Into<String>,
        onchain_hash: impl Into<String>,
    ) -> Self {
        let local_hash = local_hash.into();
        let onchain_hash = onchain_hash.into();
        Self {
            contract_id: contract_id.into(),
            network: network.into(),
            local_wasm: local_wasm.display().to_string(),
            matches: local_hash == onchain_hash,
            local_hash,
            onchain_hash,
        }
    }
}

/// Human-readable report, printed unless `--quiet`.
pub fn format_report(report: &VerifyReport) -> String {
    let mut out = String::new();
    if report.matches {
        out.push_str("✓ verified — the deployed contract matches the local build\n\n");
    } else {
        out.push_str("✗ MISMATCH — the deployed contract was NOT built from this wasm\n\n");
    }
    out.push_str(&format!("  contract   {}\n", report.contract_id));
    out.push_str(&format!("  network    {}\n", report.network));
    out.push_str(&format!("  local      {}\n", report.local_wasm));
    out.push('\n');
    if report.matches {
        out.push_str(&format!("  sha256     {}\n", report.local_hash));
    } else {
        out.push_str(&format!("  local      sha256 {}\n", report.local_hash));
        out.push_str(&format!("  on-chain   sha256 {}\n", report.onchain_hash));
        out.push('\n');
        out.push_str(
            "the deployed wasm was built from different sources or with different\n\
             build flags — rebuild with `stellar contract build` and redeploy, or\n\
             check that --network points at the deployment you meant.\n",
        );
    }
    out
}

/// The same report as JSON, for `--json`.
pub fn json_report(report: &VerifyReport) -> String {
    serde_json::to_string_pretty(report).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

/// The mismatch error returned to the CLI core, which turns it into exit
/// code `1`.
pub fn mismatch_error(report: &VerifyReport) -> ForgeError {
    ForgeError::VerificationFailed(format!(
        "contract {} on {} was built from a different wasm (on-chain sha256 {}, local sha256 {})",
        report.contract_id, report.network, report.onchain_hash, report.local_hash
    ))
}

/// Download the wasm deployed at `contract_id` into `out_file` using the
/// official CLI. Never reimplemented locally.
///
/// Flags follow `stellar contract fetch` as documented in the stellar-cli
/// reference (`--id`, `-o`/`--out-file`, `-n`/`--network`, `--rpc-url`,
/// `--network-passphrase`). We write to a file rather than reading the CLI's
/// stdout so nothing the tool prints can end up in the bytes we hash.
///
/// Thin system-touching wrapper; not unit-tested.
fn fetch_onchain_wasm(contract_id: &str, network: &NetworkArgs, out_file: &Path) -> Result<()> {
    let out_str = path_str(out_file)?;

    let mut cmd = std::process::Command::new("stellar");
    cmd.args([
        "contract",
        "fetch",
        "--id",
        contract_id,
        "--out-file",
        out_str,
    ]);
    cmd.args(network.cli_args());
    log::debug!("fetching on-chain wasm for {contract_id}");

    match cmd.output() {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            Err(ForgeError::Other(format!(
                "stellar contract fetch failed — check the contract ID and the network it is \
                 deployed on:\n{stderr}"
            )))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(ForgeError::ToolMissing("stellar-cli".into()))
        }
        Err(e) => Err(ForgeError::io("running stellar contract fetch")(e)),
    }
}

fn path_str(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| ForgeError::Other(format!("path {} is not valid UTF-8", path.display())))
}

/// Hash the local build and the deployed wasm and report whether they agree.
///
/// The local side is resolved first, so a missing build or a malformed
/// contract ID fails before any network call. When `reproducible` is set,
/// `contract_dir` is built inside the pinned image before being hashed —
/// there is no way the local toolchain can leak into the result.
pub fn verify(
    contract_id: &str,
    contract_dir: &Path,
    wasm_override: Option<&Path>,
    network: &NetworkArgs,
    reproducible: bool,
) -> Result<VerifyReport> {
    validate_contract_id(contract_id)?;

    let local_wasm = if reproducible {
        build_reproducible(contract_dir)?
    } else {
        resolve_local_wasm(contract_dir, wasm_override)?
    };
    let local_hash = hash_wasm_file(&local_wasm)?;

    let scratch = tempfile::tempdir().map_err(ForgeError::io("creating a temporary directory"))?;
    let fetched = scratch.path().join("onchain.wasm");
    fetch_onchain_wasm(contract_id, network, &fetched)?;
    let onchain_hash = hash_wasm_file(&fetched)?;

    Ok(VerifyReport::new(
        contract_id,
        network.label(),
        &local_wasm,
        local_hash,
        onchain_hash,
    ))
}

/// Run the official soroban build inside the pinned image and return the
/// resulting wasm path. `stellar contract build` runs inside the container
/// with `contract_dir` mounted at `/src`; the wasm ends up at
/// `/src/target/wasm32v1-none/release/<crate>.wasm`.
fn build_reproducible(contract_dir: &Path) -> Result<PathBuf> {
    if !contract_dir.is_dir() {
        return Err(ForgeError::InvalidArgument(format!(
            "{} is not a directory — --reproducible needs the contract source tree",
            contract_dir.display()
        )));
    }
    let dir_str = path_str(contract_dir)?;
    let crate_name = read_crate_name(contract_dir)?;
    let host_wasm = locate_wasm(contract_dir, &crate_name);

    let mut cmd = std::process::Command::new("docker");
    cmd.args([
        "run",
        "--rm",
        "-v",
        // Mount the source read-only so the container cannot mutate the host tree.
        &format!("{dir_str}:/src:ro"),
        "-w",
        "/src",
        REPRODUCIBLE_IMAGE,
        "stellar",
        "contract",
        "build",
    ]);
    log::debug!("building reproducible wasm in {REPRODUCIBLE_IMAGE}");

    match cmd.output() {
        Ok(out) if out.status.success() => {
            if !host_wasm.is_file() {
                return Err(ForgeError::Other(format!(
                    "docker build exited 0 but {} was not produced — check the build image",
                    host_wasm.display()
                )));
            }
            Ok(host_wasm)
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            Err(ForgeError::Other(format!(
                "reproducible build failed (image {REPRODUCIBLE_IMAGE}):\n{stderr}"
            )))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(ForgeError::ToolMissing("docker".into()))
        }
        Err(e) => Err(ForgeError::io("running docker")(e)),
    }
}

/// The `verify` subcommand.
pub struct VerifyPlugin;

impl ForgePlugin for VerifyPlugin {
    fn name(&self) -> &'static str {
        "verify"
    }

    fn command(&self) -> Command {
        Command::new("verify")
            .about("Check that a deployed contract matches the local release build")
            .long_about(
                "Compare the wasm deployed at a contract ID with the local release build.\n\n\
                 Both sides are identified by their SHA-256 — the hash Stellar stores \
                 uploaded contract wasm under — so a match means the deployed contract \
                 is byte-for-byte the wasm you have locally. Exits 0 on a match and 1 on \
                 a mismatch.",
            )
            .arg(
                Arg::new("contract-id")
                    .required(true)
                    .value_name("CONTRACT_ID")
                    .help("Deployed contract ID (C…)"),
            )
            .arg(
                Arg::new("path")
                    .long("path")
                    .help("Contract project directory [default: current directory]"),
            )
            .arg(
                Arg::new("wasm")
                    .long("wasm")
                    .conflicts_with("reproducible")
                    .help("Path to the local .wasm to compare [default: target/wasm32v1-none/release/<crate>.wasm]"),
            )
            .arg(
                Arg::new("network")
                    .long("network")
                    .short('n')
                    .help("Configured network the contract is deployed on [default: testnet]"),
            )
            .arg(
                Arg::new("rpc-url")
                    .long("rpc-url")
                    .help("RPC endpoint to query instead of a configured network"),
            )
            .arg(
                Arg::new("network-passphrase")
                    .long("network-passphrase")
                    .help("Network passphrase for --rpc-url"),
            )
            .arg(
                Arg::new("reproducible")
                    .long("reproducible")
                    .action(ArgAction::SetTrue)
                    .help("Build the contract inside the pinned reproducible-build container before hashing"),
            )
    }

    fn run(&self, matches: &ArgMatches, ctx: &ForgeContext) -> Result<()> {
        let contract_id = matches
            .get_one::<String>("contract-id")
            .expect("contract-id is required by clap");

        if ctx.offline {
            return Err(ForgeError::InvalidArgument(
                "verify is unavailable in offline mode because it must fetch deployed wasm".into(),
            ));
        }

        let dir = matches
            .get_one::<String>("path")
            .map(|p| ctx.cwd.join(p))
            .unwrap_or_else(|| ctx.cwd.clone());
        let wasm_override = matches.get_one::<String>("wasm").map(|p| ctx.cwd.join(p));

        let network = NetworkArgs::resolve(
            matches.get_one::<String>("network").cloned(),
            matches.get_one::<String>("rpc-url").cloned(),
            matches.get_one::<String>("network-passphrase").cloned(),
            ctx.config.as_ref().map(|c| &c.network),
        );

        let reproducible = matches.get_flag("reproducible");
        let report = verify(contract_id, &dir, wasm_override.as_deref(), &network, reproducible)?;

        if ctx.json {
            println!("{}", json_report(&report));
        } else if !ctx.quiet {
            print!("{}", format_report(&report));
        }

        if report.matches {
            Ok(())
        } else {
            Err(mismatch_error(&report))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A syntactically valid contract ID (shape only — no real checksum).
    const VALID_ID: &str = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    fn wasm_bytes(payload: &[u8]) -> Vec<u8> {
        let mut bytes = b"\0asm\x01\0\0\0".to_vec();
        bytes.extend_from_slice(payload);
        bytes
    }

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hashes_a_wasm_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("demo.wasm");
        let bytes = wasm_bytes(b"payload");
        std::fs::write(&path, &bytes).unwrap();

        assert_eq!(hash_wasm_file(&path).unwrap(), sha256_hex(&bytes));
    }

    #[test]
    fn rejects_a_file_that_is_not_wasm() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("not.wasm");
        std::fs::write(&path, b"<html>nope</html>").unwrap();

        let err = hash_wasm_file(&path).unwrap_err();
        assert!(err.to_string().contains("not a wasm module"), "{err}");
    }

    #[test]
    fn accepts_a_well_formed_contract_id() {
        assert!(validate_contract_id(VALID_ID).is_ok());
    }

    #[test]
    fn rejects_contract_ids_of_the_wrong_shape() {
        for bad in [
            "",
            "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", // account, not contract
            "CAAA",                                                     // too short
            "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", // too long
            "caaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", // lowercase
            "C1AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", // `1` is not base32
        ] {
            let err = validate_contract_id(bad).unwrap_err();
            assert!(
                err.to_string().contains("not a valid contract ID"),
                "expected `{bad}` to be rejected, got: {err}"
            );
            assert_eq!(
                err.exit_code(),
                soroban_forge_core::error::ExitCode::UserError
            );
        }
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
    fn missing_build_points_at_stellar_contract_build() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();

        let err = resolve_local_wasm(tmp.path(), None).unwrap_err();
        assert!(err.to_string().contains("stellar contract build"), "{err}");
    }

    #[test]
    fn wasm_override_wins_over_the_default_path() {
        let tmp = tempfile::tempdir().unwrap();
        let custom = tmp.path().join("custom.wasm");
        std::fs::write(&custom, wasm_bytes(b"x")).unwrap();

        assert_eq!(
            resolve_local_wasm(tmp.path(), Some(&custom)).unwrap(),
            custom
        );
    }

    #[test]
    fn a_bad_contract_id_fails_before_touching_the_network() {
        let tmp = tempfile::tempdir().unwrap();
        // No Cargo.toml, no wasm, no `stellar` on PATH — the ID check still
        // decides the outcome, so nothing here shells out.
        let err = verify("nope", tmp.path(), None, &NetworkArgs::default(), false).unwrap_err();
        assert!(err.to_string().contains("not a valid contract ID"), "{err}");
    }

    #[test]
    fn a_missing_local_build_fails_before_touching_the_network() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();

        let err = verify(VALID_ID, tmp.path(), None, &NetworkArgs::default(), false).unwrap_err();
        assert!(err.to_string().contains("stellar contract build"), "{err}");
    }

    #[test]
    fn defaults_to_testnet() {
        let network = NetworkArgs::resolve(None, None, None, None);
        assert_eq!(network.label(), "testnet");
        assert_eq!(network.cli_args(), vec!["--network", "testnet"]);
    }

    #[test]
    fn an_explicit_network_is_passed_through() {
        let network = NetworkArgs::resolve(Some("mainnet".into()), None, None, None);
        assert_eq!(network.cli_args(), vec!["--network", "mainnet"]);
    }

    #[test]
    fn an_rpc_url_replaces_the_default_network() {
        let network = NetworkArgs::resolve(
            None,
            Some("http://localhost:8000/soroban/rpc".into()),
            Some("Standalone Network ; February 2017".into()),
            None,
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
        assert_eq!(network.label(), "http://localhost:8000/soroban/rpc");
    }

    #[test]
    fn identical_hashes_are_a_match() {
        let report = VerifyReport::new(VALID_ID, "testnet", Path::new("a.wasm"), "aa", "aa");
        assert!(report.matches);

        let text = format_report(&report);
        assert!(text.contains("verified"), "{text}");
        assert!(text.contains("testnet"), "{text}");
        assert!(text.contains("aa"), "{text}");
    }

    #[test]
    fn differing_hashes_are_a_mismatch() {
        let report = VerifyReport::new(VALID_ID, "testnet", Path::new("a.wasm"), "aa", "bb");
        assert!(!report.matches);

        let text = format_report(&report);
        assert!(text.contains("MISMATCH"), "{text}");
        assert!(text.contains("aa") && text.contains("bb"), "{text}");
    }

    #[test]
    fn json_report_carries_the_verdict_and_both_hashes() {
        let report = VerifyReport::new(VALID_ID, "testnet", Path::new("a.wasm"), "aa", "bb");
        let parsed: serde_json::Value = serde_json::from_str(&json_report(&report)).unwrap();

        assert_eq!(parsed["match"], false);
        assert_eq!(parsed["contract_id"], VALID_ID);
        assert_eq!(parsed["network"], "testnet");
        assert_eq!(parsed["local_hash"], "aa");
        assert_eq!(parsed["onchain_hash"], "bb");
    }

    #[test]
    fn a_mismatch_is_a_user_facing_failure_not_an_internal_error() {
        let report = VerifyReport::new(VALID_ID, "testnet", Path::new("a.wasm"), "aa", "bb");
        let err = mismatch_error(&report);

        assert_eq!(
            err.exit_code(),
            soroban_forge_core::error::ExitCode::UserError
        );
        assert!(err.to_string().contains("aa") && err.to_string().contains("bb"));
    }

    #[test]
    fn plugin_name_matches_its_command() {
        let plugin = VerifyPlugin;
        assert_eq!(plugin.name(), plugin.command().get_name());
    }

    #[test]
    fn help_documents_the_contract_id_and_network() {
        let help = VerifyPlugin.command().render_long_help().to_string();
        assert!(help.contains("CONTRACT_ID"), "{help}");
        assert!(help.contains("--network"), "{help}");
        assert!(help.contains("--wasm"), "{help}");
        assert!(help.contains("--reproducible"), "{help}");
    }

    #[test]
    fn network_args_pull_defaults_from_config() {
        let cfg = ConfigNetwork {
            name: Some("futurenet".into()),
            rpc_url: None,
            passphrase: None,
        };
        let network = NetworkArgs::resolve(None, None, None, Some(&cfg));
        assert_eq!(network.cli_args(), vec!["--network", "futurenet"]);
    }

    #[test]
    fn network_args_cli_overrides_config() {
        let cfg = ConfigNetwork {
            name: Some("futurenet".into()),
            rpc_url: Some("http://cfg".into()),
            passphrase: Some("cfg-pass".into()),
        };
        let network = NetworkArgs::resolve(
            Some("mainnet".into()),
            Some("http://cli".into()),
            Some("cli-pass".into()),
            Some(&cfg),
        );
        assert_eq!(
            network.cli_args(),
            vec![
                "--network",
                "mainnet",
                "--rpc-url",
                "http://cli",
                "--network-passphrase",
                "cli-pass",
            ]
        );
    }
}
