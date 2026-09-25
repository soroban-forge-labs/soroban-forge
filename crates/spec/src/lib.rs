//! # soroban-forge-spec
//!
//! `soroban-forge spec` — dump the interface of a built contract: every
//! entrypoint with its argument and return types, plus the custom types
//! (structs, enums, errors) the interface refers to.
//!
//! The interface lives in the `contractspecv0` custom section of the built
//! wasm as XDR. Per soroban-forge's "wrap, don't reimplement" rule this
//! module does **not** decode that XDR itself — it shells out to the official
//! `stellar contract info interface` and only handles:
//!
//! - locating the built `.wasm` for a scaffolded project
//!   (`target/wasm32v1-none/release/<crate_name>.wasm`, the same
//!   `wasm32v1-none` layout `bindings ts`, `verify` and `doctor` expect), or
//!   using `--wasm <path>` when given
//! - asking for the representation the caller wants — the Rust-style listing
//!   for humans, raw JSON under the global `--json` flag
//! - surfacing a friendly error (pointing at `soroban-forge doctor`) when
//!   `stellar-cli` isn't on `PATH`
//!
//! Nothing here touches the network, so `spec` works under `--offline`,
//! unless `spec diff` is asked to compare a deployed contract by ID.
//!
//! `soroban-forge spec diff <old> <new>` compares two interfaces — each a
//! wasm file, a spec JSON file (as `spec --json` writes), or a deployed
//! contract ID — and classifies the differences as breaking (a removed
//! entrypoint, or one whose signature changed) or additive (a new
//! entrypoint), exiting `1` on a breaking change so CI can gate a release on
//! interface stability. See [`diff_specs`].

use std::path::{Path, PathBuf};

use clap::{Arg, ArgMatches, Command};
use serde::{Deserialize, Serialize};
use soroban_forge_core::{ForgeContext, ForgeError, ForgePlugin, Result};

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
/// Deliberately duplicated rather than shared with `bindings ts` / `verify`:
/// modules depend only on `soroban-forge-core`, never on each other.
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

/// Resolve which wasm to read the spec from: `wasm_override` when given,
/// otherwise the release build of the cargo project in `dir`. Errors when the
/// file is not there, pointing at `stellar contract build`.
pub fn resolve_wasm(dir: &Path, wasm_override: Option<&Path>) -> Result<PathBuf> {
    let wasm_path = match wasm_override {
        Some(path) => path.to_path_buf(),
        None => {
            let crate_name = read_crate_name(dir)?;
            locate_wasm(dir, &crate_name)
        }
    };

    if !wasm_path.is_file() {
        return Err(ForgeError::InvalidArgument(format!(
            "no built wasm found at {} — run `stellar contract build` first (or pass --wasm)",
            wasm_path.display()
        )));
    }
    Ok(wasm_path)
}

/// Which representation of the interface to print.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecFormat {
    /// Rust-style listing: one `fn` per entrypoint plus the custom types.
    Rust,
    /// The same spec as JSON, for editors and scripts.
    Json,
}

impl SpecFormat {
    /// Value to pass to the CLI's `--output` flag.
    ///
    /// `json-formatted` rather than `json`: the multiline form parses
    /// identically and matches the pretty-printed JSON every other
    /// soroban-forge subcommand emits.
    pub fn cli_output(self) -> &'static str {
        match self {
            SpecFormat::Rust => "rust",
            SpecFormat::Json => "json-formatted",
        }
    }

    /// `--json` picks [`SpecFormat::Json`]; everything else is the human listing.
    pub fn from_json_flag(json: bool) -> Self {
        if json {
            SpecFormat::Json
        } else {
            SpecFormat::Rust
        }
    }
}

/// The `stellar` arguments used to read a wasm's interface.
///
/// Kept as a pure function so the command line we build is unit-tested
/// without a `stellar` binary present.
///
/// Flags follow `stellar contract info interface` as of stellar-cli 27.0.0
/// (`--wasm <PATH>`, `--output <rust|xdr-base64|json|json-formatted>`); we
/// depend on the official CLI's interface rather than decoding the spec XDR
/// ourselves.
pub fn spec_cli_args(wasm: &str, format: SpecFormat) -> Vec<String> {
    vec![
        "contract".to_string(),
        "info".to_string(),
        "interface".to_string(),
        "--wasm".to_string(),
        wasm.to_string(),
        "--output".to_string(),
        format.cli_output().to_string(),
    ]
}

/// Ask the official CLI for the interface of `wasm` and return its stdout.
///
/// Thin system-touching wrapper; not unit-tested.
fn run_stellar_info(wasm: &Path, format: SpecFormat) -> Result<String> {
    let wasm_str = wasm.to_str().ok_or_else(|| {
        ForgeError::Other(format!("wasm path {} is not valid UTF-8", wasm.display()))
    })?;

    log::debug!("reading contract interface from {}", wasm.display());
    let result = std::process::Command::new("stellar")
        .args(spec_cli_args(wasm_str, format))
        .output();

    match result {
        Ok(out) if out.status.success() => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            Err(ForgeError::Other(format!(
                "stellar contract info interface failed — is {} a contract built with \
                 `stellar contract build`?\n{stderr}",
                wasm.display()
            )))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(ForgeError::ToolMissing("stellar-cli".into()))
        }
        Err(e) => Err(ForgeError::io("running stellar contract info interface")(e)),
    }
}

/// Locate the contract's wasm and return `(wasm_path, interface)` in the
/// requested representation.
pub fn dump_interface(
    contract_dir: &Path,
    wasm_override: Option<&Path>,
    format: SpecFormat,
) -> Result<(PathBuf, String)> {
    let wasm = resolve_wasm(contract_dir, wasm_override)?;
    let interface = run_stellar_info(&wasm, format)?;
    Ok((wasm, interface))
}

/// Header printed above the human listing (suppressed by `--quiet`).
pub fn format_header(wasm: &Path) -> String {
    format!("contract interface — {}\n\n", wasm.display())
}

// --- `spec diff`: interface stability check ---

/// How to reach the network for a `SpecArg::ContractId` source, mirroring
/// the `stellar` CLI's own options.
///
/// Deliberately duplicated rather than shared with `verify`: modules depend
/// only on `soroban-forge-core`, never on each other.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkArgs {
    pub network: Option<String>,
    pub rpc_url: Option<String>,
    pub network_passphrase: Option<String>,
}

/// Network used when neither `--network` nor `--rpc-url` is given.
pub const DEFAULT_NETWORK: &str = "testnet";

impl NetworkArgs {
    /// `from_config`, when given, supplies defaults from `forge.toml`'s
    /// `[network]` table; CLI flags override file values. With no network
    /// *and* no RPC URL we target [`DEFAULT_NETWORK`].
    pub fn resolve(
        network: Option<String>,
        rpc_url: Option<String>,
        network_passphrase: Option<String>,
        from_config: Option<&soroban_forge_core::config::NetworkConfig>,
    ) -> Self {
        let cfg = from_config.cloned().unwrap_or_default();
        let resolved_network = match (network.as_ref(), rpc_url.as_ref()) {
            (Some(name), _) => Some(name.clone()),
            (None, None) => Some(
                cfg.name
                    .clone()
                    .unwrap_or_else(|| DEFAULT_NETWORK.to_string()),
            ),
            (None, Some(_)) => cfg.name.clone(),
        };
        Self {
            network: resolved_network,
            rpc_url: rpc_url.or(cfg.rpc_url),
            network_passphrase: network_passphrase.or(cfg.passphrase),
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

/// Where a `spec diff` argument's interface comes from, auto-detected from
/// its shape: a `.json` file (as `spec --json` writes), a strkey contract ID
/// (`C` + 55 base32 characters), or otherwise a wasm file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecArg {
    File(PathBuf),
    Wasm(PathBuf),
    ContractId(String),
}

/// Length of a strkey-encoded contract ID (`C` + 55 base32 characters).
const CONTRACT_ID_LEN: usize = 56;

/// Cheap shape check — not a full strkey checksum, which is left to the
/// `stellar` CLI when the argument is actually used as a contract ID.
fn looks_like_contract_id(arg: &str) -> bool {
    arg.starts_with('C')
        && arg.chars().count() == CONTRACT_ID_LEN
        && arg
            .chars()
            .all(|c| c.is_ascii_uppercase() || ('2'..='7').contains(&c))
}

/// Classify a `spec diff` argument by its shape.
pub fn classify_spec_arg(arg: &str) -> SpecArg {
    if looks_like_contract_id(arg) {
        SpecArg::ContractId(arg.to_string())
    } else if Path::new(arg)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
    {
        SpecArg::File(PathBuf::from(arg))
    } else {
        SpecArg::Wasm(PathBuf::from(arg))
    }
}

/// Ask the official CLI for a wasm's interface as JSON.
///
/// Thin system-touching wrapper; not unit-tested.
fn read_interface_json_for_wasm(wasm: &Path) -> Result<String> {
    let wasm_str = wasm.to_str().ok_or_else(|| {
        ForgeError::Other(format!("wasm path {} is not valid UTF-8", wasm.display()))
    })?;
    let output = std::process::Command::new("stellar")
        .args([
            "contract",
            "info",
            "interface",
            "--wasm",
            wasm_str,
            "--output",
            "json",
        ])
        .output();
    match output {
        Ok(out) if out.status.success() => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        Ok(out) => Err(ForgeError::Other(format!(
            "stellar contract info interface failed for {}: {}",
            wasm.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(ForgeError::ToolMissing("stellar-cli".into()))
        }
        Err(e) => Err(ForgeError::io("running stellar contract info interface")(e)),
    }
}

/// Ask the official CLI for a deployed contract's interface as JSON.
///
/// Thin system-touching wrapper; not unit-tested.
fn read_interface_json_for_contract(contract_id: &str, network: &NetworkArgs) -> Result<String> {
    let mut args = vec![
        "contract".to_string(),
        "info".to_string(),
        "interface".to_string(),
        "--contract-id".to_string(),
        contract_id.to_string(),
        "--output".to_string(),
        "json".to_string(),
    ];
    args.extend(network.cli_args());
    let output = std::process::Command::new("stellar").args(&args).output();
    match output {
        Ok(out) if out.status.success() => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        Ok(out) => Err(ForgeError::Other(format!(
            "stellar contract info interface failed for {contract_id} — check the contract ID and \
             the network it is deployed on:\n{}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(ForgeError::ToolMissing("stellar-cli".into()))
        }
        Err(e) => Err(ForgeError::io("running stellar contract info interface")(e)),
    }
}

/// Resolve a `spec diff` argument to its interface JSON.
pub fn read_spec_json(arg: &SpecArg, network: &NetworkArgs, offline: bool) -> Result<String> {
    match arg {
        SpecArg::File(path) => std::fs::read_to_string(path)
            .map_err(ForgeError::io(format!("reading {}", path.display()))),
        SpecArg::Wasm(path) => read_interface_json_for_wasm(path),
        SpecArg::ContractId(id) => {
            if offline {
                return Err(ForgeError::InvalidArgument(format!(
                    "`{id}` looks like a contract ID, but spec diff is running with --offline — \
                     pass a wasm file or a spec JSON file instead, or drop --offline"
                )));
            }
            read_interface_json_for_contract(id, network)
        }
    }
}

/// One entrypoint, as read from `stellar contract info interface --output json`.
struct Entrypoint {
    name: String,
    signature: String,
}

/// Pull the `function_v0` entries out of a spec JSON document and render
/// each as `name(arg: type, …) -> type`.
fn parse_entrypoints(spec_json: &str) -> Result<Vec<Entrypoint>> {
    let entries: serde_json::Value = serde_json::from_str(spec_json)
        .map_err(|e| ForgeError::Other(format!("could not parse contract spec JSON: {e}")))?;
    let entries = entries
        .as_array()
        .ok_or_else(|| ForgeError::Other("contract spec JSON is not a list of entries".into()))?;

    let mut out = Vec::new();
    for entry in entries {
        let Some(func) = entry.get("function_v0") else {
            continue;
        };
        let name = func["name"].as_str().unwrap_or_default().to_string();
        let inputs = func["inputs"]
            .as_array()
            .map(|inputs| {
                inputs
                    .iter()
                    .map(|input| {
                        format!(
                            "{}: {}",
                            input["name"].as_str().unwrap_or("_"),
                            render_type(&input["type"])
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let outputs: Vec<String> = func["outputs"]
            .as_array()
            .map(|outputs| outputs.iter().map(render_type).collect())
            .unwrap_or_default();
        let signature = match outputs.as_slice() {
            [] => format!("{name}({inputs})"),
            [single] => format!("{name}({inputs}) -> {single}"),
            many => format!("{name}({inputs}) -> ({})", many.join(", ")),
        };
        out.push(Entrypoint { name, signature });
    }
    Ok(out)
}

/// Render an `ScSpecTypeDef` from the CLI's JSON as a compact type string.
/// Anything unrecognised falls back to its JSON, so a newer spec version
/// still diffs correctly — it just reads less nicely.
fn render_type(ty: &serde_json::Value) -> String {
    use serde_json::Value;

    if let Value::String(name) = ty {
        return name.clone();
    }
    let Some((kind, inner)) = ty
        .as_object()
        .filter(|o| o.len() == 1)
        .and_then(|o| o.iter().next())
    else {
        return ty.to_string();
    };
    match kind.as_str() {
        "udt" => inner["name"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| ty.to_string()),
        "vec" => format!("vec<{}>", render_type(&inner["element_type"])),
        "option" => format!("option<{}>", render_type(&inner["value_type"])),
        "map" => format!(
            "map<{}, {}>",
            render_type(&inner["key_type"]),
            render_type(&inner["value_type"])
        ),
        "result" => format!(
            "result<{}, {}>",
            render_type(&inner["ok_type"]),
            render_type(&inner["error_type"])
        ),
        "tuple" => {
            let types: Vec<String> = inner["value_types"]
                .as_array()
                .map(|types| types.iter().map(render_type).collect())
                .unwrap_or_default();
            format!("({})", types.join(", "))
        }
        "bytes_n" => format!("bytes<{}>", inner["n"]),
        _ => ty.to_string(),
    }
}

/// An entrypoint present on both sides whose signature differs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChangedEntrypoint {
    pub name: String,
    /// Signature in the old interface.
    pub old: String,
    /// Signature in the new interface.
    pub new: String,
}

/// Interface differences between an old and a new spec.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SpecDiff {
    /// Entrypoints only the new interface has — additive.
    pub added: Vec<String>,
    /// Entrypoints only the old interface has — breaking.
    pub removed: Vec<String>,
    /// Entrypoints present in both, with a different signature — breaking.
    pub changed: Vec<ChangedEntrypoint>,
}

impl SpecDiff {
    /// True when both interfaces expose identical entrypoints.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }

    /// A removed or changed entrypoint breaks callers depending on the old
    /// interface. A purely additive diff (new entrypoints only) does not.
    pub fn is_breaking(&self) -> bool {
        !self.removed.is_empty() || !self.changed.is_empty()
    }
}

/// Compare two interfaces given as `stellar contract info interface --output
/// json` documents (or the equivalent `spec --json` output), reading `old`
/// -> `new`: an entrypoint only `new` has is additive, one only `old` has or
/// whose signature changed is breaking.
pub fn diff_specs(old_json: &str, new_json: &str) -> Result<SpecDiff> {
    use std::collections::BTreeMap;

    let index = |entries: Vec<Entrypoint>| -> BTreeMap<String, String> {
        entries.into_iter().map(|e| (e.name, e.signature)).collect()
    };
    let old = index(parse_entrypoints(old_json)?);
    let new = index(parse_entrypoints(new_json)?);

    let mut diff = SpecDiff::default();
    for (name, new_sig) in &new {
        match old.get(name) {
            None => diff.added.push(new_sig.clone()),
            Some(old_sig) if old_sig != new_sig => diff.changed.push(ChangedEntrypoint {
                name: name.clone(),
                old: old_sig.clone(),
                new: new_sig.clone(),
            }),
            Some(_) => {}
        }
    }
    for (name, old_sig) in &old {
        if !new.contains_key(name) {
            diff.removed.push(old_sig.clone());
        }
    }
    Ok(diff)
}

/// Human-readable diff report.
pub fn format_diff_report(old_label: &str, new_label: &str, diff: &SpecDiff) -> String {
    let mut out = format!("comparing interfaces\n  old   {old_label}\n  new   {new_label}\n\n");
    if diff.is_empty() {
        out.push_str("✓ no differences\n");
        return out;
    }
    for sig in &diff.added {
        out.push_str(&format!("  + {sig}\n"));
    }
    for sig in &diff.removed {
        out.push_str(&format!("  - {sig}\n"));
    }
    for change in &diff.changed {
        out.push_str(&format!("  ~ {}\n", change.name));
        out.push_str(&format!("      old  {}\n", change.old));
        out.push_str(&format!("      new  {}\n", change.new));
    }
    out.push('\n');
    if diff.is_breaking() {
        out.push_str(&format!(
            "✗ BREAKING — {} removed, {} changed ({} added)\n",
            diff.removed.len(),
            diff.changed.len(),
            diff.added.len()
        ));
    } else {
        out.push_str(&format!(
            "✓ no breaking changes — {} added\n",
            diff.added.len()
        ));
    }
    out
}

/// The same diff as JSON, for `--json`.
pub fn json_diff_report(old_label: &str, new_label: &str, diff: &SpecDiff) -> String {
    let report = serde_json::json!({
        "old": old_label,
        "new": new_label,
        "added": diff.added,
        "removed": diff.removed,
        "changed": diff.changed,
        "breaking": diff.is_breaking(),
    });
    serde_json::to_string_pretty(&report).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

/// The breaking-change error returned to the CLI core, which turns it into
/// exit code `1`.
pub fn breaking_change_error(diff: &SpecDiff) -> ForgeError {
    ForgeError::VerificationFailed(format!(
        "spec diff found {} breaking change(s): {} entrypoint(s) removed, {} changed",
        diff.removed.len() + diff.changed.len(),
        diff.removed.len(),
        diff.changed.len()
    ))
}

/// Compare the interfaces named by `old` and `new` (each a wasm file, a spec
/// JSON file, or a contract ID) and classify the differences.
pub fn diff(old: &str, new: &str, network: &NetworkArgs, offline: bool) -> Result<SpecDiff> {
    let old_json = read_spec_json(&classify_spec_arg(old), network, offline)?;
    let new_json = read_spec_json(&classify_spec_arg(new), network, offline)?;
    diff_specs(&old_json, &new_json)
}

/// The `spec` subcommand.
pub struct SpecPlugin;

impl ForgePlugin for SpecPlugin {
    fn name(&self) -> &'static str {
        "spec"
    }

    fn command(&self) -> Command {
        Command::new("spec")
            .about("Print the contract interface (entrypoints and types) from the built wasm")
            .long_about(
                "Dump the interface of a built contract: every entrypoint with its \
                 argument and return types, plus the structs, enums and error enums \
                 the interface refers to.\n\n\
                 Reads the spec out of the built wasm, so run `stellar contract build` \
                 first. Pass the global --json flag for machine-readable output.",
            )
            .arg(
                Arg::new("path")
                    .long("path")
                    .help("Contract project directory [default: current directory]"),
            )
            .arg(Arg::new("wasm").long("wasm").help(
                "Path to the built .wasm [default: target/wasm32v1-none/release/<crate>.wasm]",
            ))
            .subcommand(
                Command::new("diff")
                    .about("Compare two contract interfaces and classify the differences as breaking or additive")
                    .long_about(
                        "Compare two interfaces and classify the differences.\n\n\
                         Each of OLD and NEW is auto-detected: a spec JSON file (as `spec --json` \
                         writes, must end in .json), a deployed contract ID (C…), or otherwise a \
                         wasm file.\n\n\
                         A removed entrypoint or one whose signature changed is breaking; a new \
                         entrypoint is additive. Exits 1 on a breaking change so CI can gate a \
                         release on interface stability. Pass the global --json flag for \
                         machine-readable output.",
                    )
                    .arg(
                        Arg::new("old")
                            .required(true)
                            .value_name("OLD")
                            .help("Baseline interface: a wasm file, a spec JSON file, or a contract ID"),
                    )
                    .arg(
                        Arg::new("new")
                            .required(true)
                            .value_name("NEW")
                            .help("Candidate interface: a wasm file, a spec JSON file, or a contract ID"),
                    )
                    .arg(
                        Arg::new("network")
                            .long("network")
                            .short('n')
                            .help("Configured network to query, when OLD/NEW is a contract ID [default: testnet]"),
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
                    ),
            )
    }

    fn run(&self, matches: &ArgMatches, ctx: &ForgeContext) -> Result<()> {
        if let Some(sub) = matches.subcommand_matches("diff") {
            return run_diff(sub, ctx);
        }

        let dir = matches
            .get_one::<String>("path")
            .map(|p| ctx.cwd.join(p))
            .unwrap_or_else(|| ctx.cwd.clone());
        let wasm_override = matches.get_one::<String>("wasm").map(|p| ctx.cwd.join(p));

        let format = SpecFormat::from_json_flag(ctx.json);
        let (wasm, interface) = dump_interface(&dir, wasm_override.as_deref(), format)?;

        if ctx.json {
            // The CLI already emits JSON; pass it through unchanged so the
            // spec stays byte-identical to what stellar-cli reports.
            print!("{interface}");
            if !interface.ends_with('\n') {
                println!();
            }
            return Ok(());
        }

        if !ctx.quiet {
            print!("{}", format_header(&wasm));
        }
        print!("{interface}");
        if !interface.ends_with('\n') {
            println!();
        }
        Ok(())
    }
}

fn run_diff(matches: &ArgMatches, ctx: &ForgeContext) -> Result<()> {
    let old = matches.get_one::<String>("old").expect("required by clap");
    let new = matches.get_one::<String>("new").expect("required by clap");

    let network = NetworkArgs::resolve(
        matches.get_one::<String>("network").cloned(),
        matches.get_one::<String>("rpc-url").cloned(),
        matches.get_one::<String>("network-passphrase").cloned(),
        ctx.config.as_ref().map(|c| &c.network),
    );

    let diff = diff(old, new, &network, ctx.offline)?;

    if ctx.json {
        println!("{}", json_diff_report(old, new, &diff));
    } else if !ctx.quiet {
        print!("{}", format_diff_report(old, new, &diff));
    }

    if diff.is_breaking() {
        Err(breaking_change_error(&diff))
    } else {
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
    fn reads_crate_name_and_normalizes_dashes() {
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

        let err = resolve_wasm(tmp.path(), None).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("stellar contract build"), "{msg}");
        assert!(msg.contains("demo.wasm"), "{msg}");
    }

    #[test]
    fn explicit_wasm_override_is_used_verbatim() {
        let tmp = tempfile::tempdir().unwrap();
        let wasm = tmp.path().join("custom.wasm");
        std::fs::write(&wasm, b"\0asm").unwrap();
        // No Cargo.toml in `dir` — the override must short-circuit the lookup.
        assert_eq!(resolve_wasm(tmp.path(), Some(&wasm)).unwrap(), wasm);
    }

    #[test]
    fn missing_wasm_override_is_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("nope.wasm");
        let err = resolve_wasm(tmp.path(), Some(&missing)).unwrap_err();
        assert!(err.to_string().contains("nope.wasm"), "{err}");
    }

    #[test]
    fn human_format_asks_the_cli_for_the_rust_listing() {
        assert_eq!(
            spec_cli_args("/tmp/demo.wasm", SpecFormat::Rust),
            vec![
                "contract",
                "info",
                "interface",
                "--wasm",
                "/tmp/demo.wasm",
                "--output",
                "rust"
            ]
        );
    }

    #[test]
    fn json_format_asks_the_cli_for_json() {
        let args = spec_cli_args("/tmp/demo.wasm", SpecFormat::Json);
        assert_eq!(args.last().unwrap(), "json-formatted");
    }

    #[test]
    fn json_flag_selects_the_json_format() {
        assert_eq!(SpecFormat::from_json_flag(true), SpecFormat::Json);
        assert_eq!(SpecFormat::from_json_flag(false), SpecFormat::Rust);
    }

    #[test]
    fn header_names_the_wasm_that_was_read() {
        let header = format_header(Path::new("/proj/target/demo.wasm"));
        assert!(header.starts_with("contract interface — "));
        assert!(header.contains("/proj/target/demo.wasm"));
    }

    #[test]
    fn command_exposes_path_and_wasm_flags() {
        let matches = SpecPlugin
            .command()
            .try_get_matches_from(vec!["spec", "--path", "proj", "--wasm", "a.wasm"])
            .unwrap();
        assert_eq!(
            matches.get_one::<String>("path").map(String::as_str),
            Some("proj")
        );
        assert_eq!(
            matches.get_one::<String>("wasm").map(String::as_str),
            Some("a.wasm")
        );
    }

    #[test]
    fn command_name_matches_plugin_name() {
        assert_eq!(SpecPlugin.name(), SpecPlugin.command().get_name());
    }

    // --- spec diff: argument classification ---

    #[test]
    fn classifies_a_contract_id() {
        let id = "C".to_string() + &"A".repeat(55);
        assert_eq!(classify_spec_arg(&id), SpecArg::ContractId(id));
    }

    #[test]
    fn classifies_a_json_file_case_insensitively() {
        assert_eq!(
            classify_spec_arg("old.json"),
            SpecArg::File(PathBuf::from("old.json"))
        );
        assert_eq!(
            classify_spec_arg("old.JSON"),
            SpecArg::File(PathBuf::from("old.JSON"))
        );
    }

    #[test]
    fn classifies_anything_else_as_wasm() {
        assert_eq!(
            classify_spec_arg("target/wasm32v1-none/release/demo.wasm"),
            SpecArg::Wasm(PathBuf::from("target/wasm32v1-none/release/demo.wasm"))
        );
        // Too short/wrong charset to be a contract ID, no .json extension.
        assert_eq!(
            classify_spec_arg("not-an-id"),
            SpecArg::Wasm(PathBuf::from("not-an-id"))
        );
    }

    #[test]
    fn offline_rejects_a_contract_id_before_touching_the_network() {
        let id = "C".to_string() + &"A".repeat(55);
        let err =
            read_spec_json(&SpecArg::ContractId(id), &NetworkArgs::default(), true).unwrap_err();
        assert!(err.to_string().contains("--offline"), "{err}");
    }

    #[test]
    fn a_file_source_is_read_from_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("old.json");
        std::fs::write(&path, "[]").unwrap();
        assert_eq!(
            read_spec_json(&SpecArg::File(path), &NetworkArgs::default(), false).unwrap(),
            "[]"
        );
    }

    // --- spec diff: network args ---

    #[test]
    fn network_args_default_to_testnet() {
        let network = NetworkArgs::resolve(None, None, None, None);
        assert_eq!(network.cli_args(), vec!["--network", "testnet"]);
    }

    #[test]
    fn network_args_cli_overrides_config() {
        let cfg = soroban_forge_core::config::NetworkConfig {
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
                "cli-pass"
            ]
        );
    }

    // --- spec diff: entrypoint diffing ---

    type FuncSpec<'a> = (
        &'a str,
        &'a [(&'a str, serde_json::Value)],
        Option<serde_json::Value>,
    );

    /// A spec JSON document in the shape `stellar contract info interface
    /// --output json` emits, with one `function_v0` per `(name, inputs, output)`.
    fn spec_json(funcs: &[FuncSpec]) -> String {
        let mut entries: Vec<serde_json::Value> = funcs
            .iter()
            .map(|(name, inputs, output)| {
                let inputs: Vec<_> = inputs
                    .iter()
                    .map(|(n, t)| serde_json::json!({"doc": "", "name": n, "type": t}))
                    .collect();
                let outputs: Vec<_> = output.iter().cloned().collect();
                serde_json::json!({"function_v0": {"doc": "", "name": name, "inputs": inputs, "outputs": outputs}})
            })
            .collect();
        entries.push(serde_json::json!({"udt_error_enum_v0": {"name": "Error", "cases": []}}));
        serde_json::to_string(&entries).unwrap()
    }

    #[test]
    fn diff_classifies_added_removed_and_changed() {
        use serde_json::json;
        let old = spec_json(&[
            ("balance", &[("id", json!("address"))], Some(json!("i128"))),
            (
                "burn",
                &[("from", json!("address")), ("amount", json!("i128"))],
                None,
            ),
            ("name", &[], Some(json!("string"))),
        ]);
        let new = spec_json(&[
            ("balance", &[("id", json!("address"))], Some(json!("u128"))),
            ("name", &[], Some(json!("string"))),
            (
                "mint",
                &[("to", json!("address")), ("amount", json!("i128"))],
                None,
            ),
        ]);

        let diff = diff_specs(&old, &new).unwrap();
        assert_eq!(diff.added, vec!["mint(to: address, amount: i128)"]);
        assert_eq!(diff.removed, vec!["burn(from: address, amount: i128)"]);
        assert_eq!(
            diff.changed,
            vec![ChangedEntrypoint {
                name: "balance".into(),
                old: "balance(id: address) -> i128".into(),
                new: "balance(id: address) -> u128".into(),
            }]
        );
        assert!(diff.is_breaking());
        assert!(!diff.is_empty());
    }

    #[test]
    fn identical_interfaces_are_not_breaking() {
        let spec = spec_json(&[("hello", &[("to", serde_json::json!("symbol"))], None)]);
        let diff = diff_specs(&spec, &spec).unwrap();
        assert!(diff.is_empty());
        assert!(!diff.is_breaking());
    }

    #[test]
    fn purely_additive_diff_is_not_breaking() {
        use serde_json::json;
        let old = spec_json(&[("balance", &[("id", json!("address"))], Some(json!("i128")))]);
        let new = spec_json(&[
            ("balance", &[("id", json!("address"))], Some(json!("i128"))),
            ("mint", &[("to", json!("address"))], None),
        ]);
        let diff = diff_specs(&old, &new).unwrap();
        assert!(!diff.is_breaking());
        assert_eq!(diff.added.len(), 1);
    }

    #[test]
    fn a_removed_entrypoint_is_breaking() {
        use serde_json::json;
        let old = spec_json(&[("burn", &[("from", json!("address"))], None)]);
        let new = spec_json(&[]);
        assert!(diff_specs(&old, &new).unwrap().is_breaking());
    }

    #[test]
    fn a_changed_signature_is_breaking() {
        use serde_json::json;
        let old = spec_json(&[("mint", &[("to", json!("address"))], None)]);
        let new = spec_json(&[(
            "mint",
            &[("to", json!("address")), ("amount", json!("i128"))],
            None,
        )]);
        assert!(diff_specs(&old, &new).unwrap().is_breaking());
    }

    #[test]
    fn malformed_spec_json_is_an_error_not_a_panic() {
        assert!(diff_specs("not json", "[]").is_err());
        assert!(diff_specs("{}", "[]").is_err());
    }

    #[test]
    fn renders_compound_types() {
        use serde_json::json;
        let ty = json!({"result": {
            "ok_type": {"vec": {"element_type": {"option": {"value_type": {"udt": {"name": "Offer"}}}}}},
            "error_type": {"udt": {"name": "Error"}}
        }});
        assert_eq!(render_type(&ty), "result<vec<option<Offer>>, Error>");
        assert_eq!(
            render_type(
                &json!({"map": {"key_type": "symbol", "value_type": {"bytes_n": {"n": 32}}}})
            ),
            "map<symbol, bytes<32>>"
        );
        assert_eq!(render_type(&json!({"tuple": {"value_types": []}})), "()");
        assert_eq!(render_type(&json!({"future": 1})), r#"{"future":1}"#);
    }

    // --- spec diff: reports and exit behaviour ---

    #[test]
    fn format_report_shows_breaking_verdict() {
        let diff = SpecDiff {
            added: vec!["mint(to: address)".into()],
            removed: vec!["burn(from: address)".into()],
            changed: vec![ChangedEntrypoint {
                name: "balance".into(),
                old: "balance(id: address) -> i128".into(),
                new: "balance(id: address) -> u128".into(),
            }],
        };
        let text = format_diff_report("old.wasm", "new.wasm", &diff);
        assert!(text.contains("+ mint(to: address)"), "{text}");
        assert!(text.contains("- burn(from: address)"), "{text}");
        assert!(text.contains("~ balance"), "{text}");
        assert!(text.contains("BREAKING"), "{text}");
    }

    #[test]
    fn format_report_shows_additive_only_verdict() {
        let diff = SpecDiff {
            added: vec!["mint(to: address)".into()],
            ..SpecDiff::default()
        };
        let text = format_diff_report("old.wasm", "new.wasm", &diff);
        assert!(text.contains("no breaking changes"), "{text}");
        assert!(!text.contains("BREAKING"), "{text}");
    }

    #[test]
    fn json_report_includes_the_breaking_verdict() {
        let diff = SpecDiff {
            removed: vec!["burn(from: address)".into()],
            ..SpecDiff::default()
        };
        let parsed: serde_json::Value =
            serde_json::from_str(&json_diff_report("old.wasm", "new.wasm", &diff)).unwrap();
        assert_eq!(parsed["breaking"], true);
        assert_eq!(parsed["old"], "old.wasm");
        assert_eq!(parsed["new"], "new.wasm");
        assert_eq!(parsed["removed"][0], "burn(from: address)");
    }

    #[test]
    fn breaking_change_error_exits_with_user_error_code() {
        let diff = SpecDiff {
            removed: vec!["burn(from: address)".into()],
            ..SpecDiff::default()
        };
        let err = breaking_change_error(&diff);
        assert_eq!(
            err.exit_code(),
            soroban_forge_core::error::ExitCode::UserError
        );
    }

    #[test]
    fn diff_end_to_end_from_file_sources() {
        let tmp = tempfile::tempdir().unwrap();
        let old_path = tmp.path().join("old.json");
        let new_path = tmp.path().join("new.json");
        std::fs::write(&old_path, spec_json(&[("burn", &[], None)])).unwrap();
        std::fs::write(&new_path, spec_json(&[("mint", &[], None)])).unwrap();

        let result = diff(
            old_path.to_str().unwrap(),
            new_path.to_str().unwrap(),
            &NetworkArgs::default(),
            false,
        )
        .unwrap();
        assert!(result.is_breaking());
        assert_eq!(result.added, vec!["mint()"]);
        assert_eq!(result.removed, vec!["burn()"]);
    }

    #[test]
    fn diff_subcommand_requires_old_and_new() {
        let matches = SpecPlugin
            .command()
            .try_get_matches_from(vec!["spec", "diff", "old.json", "new.json"])
            .unwrap();
        let sub = matches.subcommand_matches("diff").unwrap();
        assert_eq!(
            sub.get_one::<String>("old").map(String::as_str),
            Some("old.json")
        );
        assert_eq!(
            sub.get_one::<String>("new").map(String::as_str),
            Some("new.json")
        );
    }

    #[test]
    fn diff_help_documents_breaking_vs_additive() {
        let mut cmd = SpecPlugin.command();
        let diff_cmd = cmd.find_subcommand_mut("diff").expect("diff subcommand");
        let help = diff_cmd.render_long_help().to_string();
        assert!(help.contains("breaking"), "{help}");
        assert!(help.contains("additive"), "{help}");
        assert!(help.contains("Exits 1"), "{help}");
    }
}
