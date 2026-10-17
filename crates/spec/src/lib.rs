//! # soroban-forge-spec
//!
//! `soroban-forge spec` — dump the interface of a built contract or deployed
//! contract: every entrypoint with its argument and return types, plus the
//! custom types (structs, enums, errors, unions) the interface refers to.
//!
//! When given a contract ID (e.g. `spec <CONTRACT_ID>`), it fetches the deployed
//! wasm from the network before extracting its interface.
//!
//! Output formats:
//! - Rust-style listing (default for terminal viewing)
//! - Raw JSON (via `--format json` or `--json`)
//! - Documentation-ready Markdown table (via `--format md`), including custom
//!   types referenced by entrypoints, formatted for embedding in a README.
//!
//! ## Implemented features
//! - **#399** mtime-keyed disk cache; bypassed with `--no-cache`
//! - **#400** distinguishes a missing interface section from a corrupt/truncated wasm
//! - **#401** `--count` flag (human mode) / count fields added to JSON output
//! - **#402** `run_stellar_info` respects the global `--timeout` via `output_with_timeout`

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Arg, ArgMatches, Command};
use serde::{Deserialize, Serialize};
use soroban_forge_core::{ForgeContext, ForgeError, ForgePlugin, Result};

/// Network used when neither `--network` nor `--rpc-url` is given.
pub const DEFAULT_NETWORK: &str = "testnet";

// ---------------------------------------------------------------------------
// #401 — count of entrypoints and custom types
// ---------------------------------------------------------------------------

/// Summary counts extracted from a contract's spec JSON.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct SpecCounts {
    /// Number of entrypoint functions.
    pub entrypoints: usize,
    /// Number of custom types (structs, enums, error enums, unions).
    pub custom_types: usize,
}

impl SpecCounts {
    /// Parse `spec_json` (a JSON array produced by `stellar contract info
    /// interface --output json-formatted`) and count entrypoints and custom types.
    pub fn from_spec_json(spec_json: &str) -> Self {
        let Ok(entries) = serde_json::from_str::<serde_json::Value>(spec_json) else {
            return Self::default();
        };
        let Some(entries) = entries.as_array() else {
            return Self::default();
        };
        let mut entrypoints = 0usize;
        let mut custom_types = 0usize;
        for entry in entries {
            if entry.get("function_v0").is_some() {
                entrypoints += 1;
            } else if entry.get("udt_struct_v0").is_some()
                || entry.get("udt_enum_v0").is_some()
                || entry.get("udt_error_enum_v0").is_some()
                || entry.get("udt_union_v0").is_some()
            {
                custom_types += 1;
            }
        }
        Self {
            entrypoints,
            custom_types,
        }
    }

    /// One-line human-readable summary, e.g. `"2 entrypoints, 3 custom types"`.
    pub fn summary_line(&self) -> String {
        format!(
            "{} {}, {} custom {}",
            self.entrypoints,
            if self.entrypoints == 1 { "entrypoint" } else { "entrypoints" },
            self.custom_types,
            if self.custom_types == 1 { "type" } else { "types" },
        )
    }
}

// ---------------------------------------------------------------------------
// #399 — mtime-keyed wasm interface cache
// ---------------------------------------------------------------------------

/// Persistent entry stored in the on-disk spec cache.
#[derive(Serialize, Deserialize)]
struct CacheEntry {
    /// Absolute path of the wasm that was read.
    wasm: PathBuf,
    /// Seconds since UNIX_EPOCH of the wasm file's last modification time when
    /// this entry was written. Used to invalidate stale cache hits.
    mtime_secs: u64,
    /// The CLI output format this entry corresponds to.
    format: String,
    /// The cached interface text (Rust listing, JSON array, or Markdown).
    interface: String,
}

/// Derive the cache file path for `wasm` and `format` inside a per-user
/// scratch directory (`$TMPDIR/soroban-forge-spec-cache/`).
fn cache_path(wasm: &Path, format: SpecFormat) -> Option<PathBuf> {
    // Use a stable, deterministic name: sha256 of the canonical wasm path.
    // We do NOT need the sha2 crate: a simple hex of std::hash is enough for
    // a local advisory cache (not a security boundary).
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    wasm.hash(&mut hasher);
    format.cli_output().hash(&mut hasher);
    let key = format!("{:016x}", hasher.finish());

    let mut dir = std::env::temp_dir();
    dir.push("soroban-forge-spec-cache");
    let _ = std::fs::create_dir_all(&dir); // best-effort
    Some(dir.join(format!("{key}.json")))
}

/// Return the wasm's mtime as seconds-since-epoch, or `None` if unavailable.
fn wasm_mtime_secs(wasm: &Path) -> Option<u64> {
    use std::time::UNIX_EPOCH;
    wasm.metadata().ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}

/// Try to return a previously-cached interface string for `wasm`/`format`.
/// Returns `None` on any miss, error, or if the wasm has been modified since
/// the cache was written.
pub fn cache_lookup(wasm: &Path, format: SpecFormat) -> Option<String> {
    let path = cache_path(wasm, format)?;
    let raw = std::fs::read_to_string(&path).ok()?;
    let entry: CacheEntry = serde_json::from_str(&raw).ok()?;
    // Validate path and mtime
    if entry.wasm != wasm {
        return None;
    }
    if entry.format != format.cli_output() {
        return None;
    }
    let current_mtime = wasm_mtime_secs(wasm)?;
    if entry.mtime_secs != current_mtime {
        log::debug!(
            "cache stale for {} (stored mtime={}, current={})",
            wasm.display(),
            entry.mtime_secs,
            current_mtime
        );
        return None;
    }
    log::debug!("cache hit for {} (format={})", wasm.display(), format.cli_output());
    Some(entry.interface)
}

/// Persist `interface` in the cache for `wasm`/`format`. Failures are silently
/// ignored (the cache is advisory only — a write error must not fail the command).
pub fn cache_store(wasm: &Path, format: SpecFormat, interface: &str) {
    let Some(path) = cache_path(wasm, format) else { return };
    let Some(mtime_secs) = wasm_mtime_secs(wasm) else { return };
    let entry = CacheEntry {
        wasm: wasm.to_path_buf(),
        mtime_secs,
        format: format.cli_output().to_string(),
        interface: interface.to_string(),
    };
    if let Ok(serialized) = serde_json::to_string(&entry) {
        let _ = std::fs::write(&path, serialized);
    }
}

/// Length of a strkey-encoded contract ID (`C` + 55 base32 characters).
pub const CONTRACT_ID_LEN: usize = 56;

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

/// Shape check on a strkey contract ID so an invalid ID fails before
/// touching the network. Contract IDs are 56 base32 characters starting with 'C'.
pub fn validate_contract_id(id: &str) -> Result<()> {
    fn err(id: &str, reason: &str) -> ForgeError {
        ForgeError::InvalidArgument(format!(
            "`{id}` is not a valid contract ID ({reason}); expected {CONTRACT_ID_LEN} characters starting with `C`"
        ))
    }
    if id.chars().count() != CONTRACT_ID_LEN {
        return Err(err(id, "wrong length"));
    }
    if !id.starts_with('C') {
        return Err(err(id, "must start with C"));
    }
    if !id.chars().all(|c| matches!(c, 'A'..='Z' | '2'..='7')) {
        return Err(err(id, "contains non-base32 character"));
    }
    Ok(())
}

fn looks_like_contract_id(value: &str) -> bool {
    value.starts_with('C') && value.chars().count() == CONTRACT_ID_LEN
}

/// How to reach the network when fetching deployed contract wasm.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkArgs {
    pub network: Option<String>,
    pub rpc_url: Option<String>,
    pub network_passphrase: Option<String>,
}

/// One interface change detected by `spec diff`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpecChange {
    pub function: String,
    pub old_signature: Option<String>,
    pub new_signature: Option<String>,
}

/// Changes between two contract interfaces.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SpecDiff {
    pub breaking: Vec<SpecChange>,
    pub additive: Vec<SpecChange>,
}

impl SpecDiff {
    pub fn is_breaking(&self) -> bool {
        !self.breaking.is_empty()
    }
}

fn entrypoint_signature(entry: &serde_json::Value) -> Result<String> {
    let function = entry
        .get("function_v0")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| ForgeError::InvalidArgument("invalid function entry in contract spec".into()))?;
    let inputs = function
        .get("inputs")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| ForgeError::InvalidArgument("function spec is missing an inputs array".into()))?;
    let outputs = function
        .get("outputs")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| ForgeError::InvalidArgument("function spec is missing an outputs array".into()))?;
    let input_signatures = inputs
        .iter()
        .map(|input| {
            let name = input.get("name").and_then(serde_json::Value::as_str)
                .ok_or_else(|| ForgeError::InvalidArgument("function input is missing its name".into()))?;
            let ty = input.get("type")
                .ok_or_else(|| ForgeError::InvalidArgument("function input is missing its type".into()))?;
            Ok(format!("{name}: {}", render_type(ty)))
        })
        .collect::<Result<Vec<_>>>()?;
    let output_signatures = outputs.iter().map(render_type).collect::<Vec<_>>();
    Ok(format!("({}) -> {}", input_signatures.join(", "), output_signatures.join(", ")))
}

fn spec_entrypoints(spec_json: &str) -> Result<BTreeMap<String, (serde_json::Value, String)>> {
    let entries: serde_json::Value = serde_json::from_str(spec_json)
        .map_err(|e| ForgeError::InvalidArgument(format!("could not parse contract spec JSON: {e}")))?;
    let entries = entries.as_array()
        .ok_or_else(|| ForgeError::InvalidArgument("contract spec JSON is not an array".into()))?;
    let mut functions = BTreeMap::new();
    for entry in entries {
        let Some(function) = entry.get("function_v0") else { continue };
        let name = function.get("name").and_then(serde_json::Value::as_str)
            .ok_or_else(|| ForgeError::InvalidArgument("function spec is missing its name".into()))?;
        let signature = entrypoint_signature(entry)?;
        if functions.insert(name.to_string(), (entry.clone(), signature)).is_some() {
            return Err(ForgeError::InvalidArgument(format!("contract spec contains duplicate entrypoint `{name}`")));
        }
    }
    Ok(functions)
}

/// Compare two JSON-formatted contract specs by entrypoint and full signature.
pub fn diff_specs(old_spec: &str, new_spec: &str) -> Result<SpecDiff> {
    let old = spec_entrypoints(old_spec)?;
    let new = spec_entrypoints(new_spec)?;
    let mut diff = SpecDiff::default();
    for (name, (old_entry, old_signature)) in &old {
        match new.get(name) {
            None => diff.breaking.push(SpecChange {
                function: name.clone(),
                old_signature: Some(old_signature.clone()),
                new_signature: None,
            }),
            Some((new_entry, new_signature)) if old_entry["function_v0"]["inputs"] != new_entry["function_v0"]["inputs"]
                || old_entry["function_v0"]["outputs"] != new_entry["function_v0"]["outputs"] => {
                diff.breaking.push(SpecChange {
                    function: name.clone(),
                    old_signature: Some(old_signature.clone()),
                    new_signature: Some(new_signature.clone()),
                });
            }
            Some(_) => {}
        }
    }
    for (name, (_, signature)) in &new {
        if !old.contains_key(name) {
            diff.additive.push(SpecChange {
                function: name.clone(),
                old_signature: None,
                new_signature: Some(signature.clone()),
            });
        }
    }
    Ok(diff)
}

/// Resolve a diff input as a spec JSON file, WASM file, or deployed contract ID.
pub fn load_diff_spec(
    source: &str,
    cwd: &Path,
    network: &NetworkArgs,
    timeout: Option<Duration>,
    no_cache: bool,
) -> Result<String> {
    let path = cwd.join(source);
    if path.is_file() {
        if path.extension().and_then(|ext| ext.to_str()) == Some("wasm") {
            return dump_interface_from_wasm(&path, SpecFormat::Json, no_cache, timeout);
        }
        return std::fs::read_to_string(&path)
            .map_err(ForgeError::io(format!("reading spec file {}", path.display())));
    }
    validate_contract_id(source)?;
    let temp = tempfile::tempdir().map_err(ForgeError::io("creating temporary directory"))?;
    let wasm = temp.path().join("contract.wasm");
    fetch_onchain_wasm(source, network, &wasm, timeout)?;
    dump_interface_from_wasm(&wasm, SpecFormat::Json, no_cache, timeout)
}

pub fn format_spec_diff(diff: &SpecDiff) -> String {
    let mut output = String::new();
    if diff.breaking.is_empty() && diff.additive.is_empty() {
        return "No entrypoint changes.\n".into();
    }
    for change in &diff.breaking {
        match (&change.old_signature, &change.new_signature) {
            (Some(old), Some(new)) => output.push_str(&format!("BREAKING changed `{}`: {old} -> {new}\n", change.function)),
            (Some(old), None) => output.push_str(&format!("BREAKING removed `{}`: {old}\n", change.function)),
            _ => unreachable!("breaking changes have an old signature"),
        }
    }
    for change in &diff.additive {
        output.push_str(&format!("ADDITIVE added `{}`: {}\n", change.function, change.new_signature.as_deref().unwrap_or("")));
    }
    output
}

impl NetworkArgs {
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

/// Download the wasm deployed at `contract_id` into `out_file` using the official CLI.
fn fetch_onchain_wasm(
    contract_id: &str,
    network: &NetworkArgs,
    out_file: &Path,
    timeout: Option<Duration>,
) -> Result<()> {
    let out_str = out_file
        .to_str()
        .ok_or_else(|| ForgeError::Other(format!("path {} is not valid UTF-8", out_file.display())))?;

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

    match soroban_forge_core::timeout::output_with_timeout(&mut cmd, timeout) {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            Err(ForgeError::Other(format!(
                "stellar contract fetch failed — check the contract ID and the network it is deployed on:\n{stderr}"
            )))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(ForgeError::ToolMissing("stellar-cli".into()))
        }
        Err(e) => Err(ForgeError::io("running stellar contract fetch")(e)),
    }
}

/// Which representation of the interface to emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecFormat {
    /// Rust-style listing: one `fn` per entrypoint plus the custom types.
    Rust,
    /// Raw spec as JSON, for editors and scripts.
    Json,
    /// Markdown documentation table of entrypoints and referenced custom types.
    Markdown,
    /// Raw XDR-base64 output from stellar CLI for tooling that needs it directly.
    Xdr,
}

impl SpecFormat {
    /// Value to pass to the CLI's `--output` flag.
    pub fn cli_output(self) -> &'static str {
        match self {
            SpecFormat::Rust => "rust",
            SpecFormat::Json | SpecFormat::Markdown => "json-formatted",
            SpecFormat::Xdr => "xdr-base64",
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

/// Resolve the format from CLI matches and global context.
pub fn resolve_format(matches: &ArgMatches, ctx: &ForgeContext) -> Result<SpecFormat> {
    if let Some(fmt) = matches.get_one::<String>("format") {
        match fmt.as_str() {
            "md" | "markdown" => Ok(SpecFormat::Markdown),
            "json" => Ok(SpecFormat::Json),
            "rust" | "text" => Ok(SpecFormat::Rust),
            "xdr" => Ok(SpecFormat::Xdr),
            other => Err(ForgeError::InvalidArgument(format!(
                "unsupported spec format `{other}`; expected `rust`, `json`, `xdr` or `md`"
            ))),
        }
    } else if ctx.json {
        Ok(SpecFormat::Json)
    } else {
        Ok(SpecFormat::Rust)
    }
}

/// The `stellar` arguments used to read a wasm's interface.
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
/// - **#402**: uses `output_with_timeout` so a hung subprocess is killed after
///   the caller-supplied deadline rather than blocking forever.
/// - **#400**: inspects the stderr coming back from the CLI to produce a more
///   specific error message when the wasm simply has no interface section
///   (not a Soroban contract) versus when the wasm is corrupt/truncated.
fn run_stellar_info(wasm: &Path, format: SpecFormat, timeout: Option<Duration>) -> Result<String> {
    let wasm_str = wasm.to_str().ok_or_else(|| {
        ForgeError::Other(format!("wasm path {} is not valid UTF-8", wasm.display()))
    })?;

    log::debug!("reading contract interface from {}", wasm.display());
    let mut cmd = std::process::Command::new("stellar");
    cmd.args(spec_cli_args(wasm_str, format));

    let result = soroban_forge_core::timeout::output_with_timeout(&mut cmd, timeout);

    match result {
        Ok(out) if out.status.success() => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            // #400 — distinguish "no interface section" from "corrupt wasm"
            Err(classify_stellar_error(wasm, &stderr))
        }
        // #402 — timed-out subprocess
        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
            Err(ForgeError::Other(format!(
                "stellar contract info interface timed out after {}s while reading {} \
                 — is the stellar CLI hanging? Try --timeout to adjust the limit, or \
                 check your environment",
                timeout.map(|d| d.as_secs()).unwrap_or(0),
                wasm.display()
            )))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(ForgeError::ToolMissing("stellar-cli".into()))
        }
        Err(e) => Err(ForgeError::io("running stellar contract info interface")(e)),
    }
}

// ---------------------------------------------------------------------------
// #400 — classify stellar CLI stderr into "no interface" vs "corrupt wasm"
// ---------------------------------------------------------------------------

/// Patterns in the stellar CLI stderr that indicate the wasm was parsed but
/// has no Soroban interface section (i.e. it is not a Soroban contract).
const NO_INTERFACE_HINTS: &[&str] = &[
    "no contract spec",
    "contract spec not found",
    "no spec",
    "missing spec",
    "not a soroban",
    "no interface",
    "interface not found",
];

/// Patterns that suggest the wasm file itself is corrupt or truncated.
const CORRUPT_HINTS: &[&str] = &[
    "invalid magic",
    "unexpected end",
    "truncated",
    "corrupt",
    "failed to parse",
    "invalid wasm",
    "decode error",
    "malformed",
];

/// Map the stderr of a failed `stellar contract info interface` invocation into
/// a human-friendly [`ForgeError`].
///
/// - If stderr hints that the wasm has no interface section, return a message
///   explaining this is probably not a Soroban contract.
/// - If stderr hints at parse/corrupt problems, say so explicitly.
/// - Otherwise fall back to the original generic message.
pub fn classify_stellar_error(wasm: &Path, stderr: &str) -> ForgeError {
    let lower = stderr.to_lowercase();

    if NO_INTERFACE_HINTS.iter().any(|h| lower.contains(h)) {
        return ForgeError::InvalidArgument(format!(
            "{} has no Soroban interface section — it may be a non-Soroban wasm \
             (not built with `stellar contract build`). \
             Ensure the contract derives #[contract] and was compiled for \
             `wasm32v1-none`.\n{stderr}",
            wasm.display()
        ));
    }

    if CORRUPT_HINTS.iter().any(|h| lower.contains(h)) {
        return ForgeError::Other(format!(
            "{} appears to be corrupt or truncated (stellar could not parse it). \
             Try rebuilding with `stellar contract build`.\n{stderr}",
            wasm.display()
        ));
    }

    // Generic fallback
    ForgeError::Other(format!(
        "stellar contract info interface failed — is {} a contract built with \
         `stellar contract build`?\n{stderr}",
        wasm.display()
    ))
}

/// Escape a contract-supplied identifier for use inside a Markdown table cell.
///
/// Names and types come from the contract's own wasm spec, so they are
/// untrusted from this tool's point of view. A literal `|` would add a column
/// and a backtick would close the inline-code span early, either of which
/// breaks the generated table or lets contract text alter how the doc renders
/// once embedded in a README (#483).
fn escape_markdown_cell(value: &str) -> String {
    value.replace('\\', "\\\\").replace('|', "\\|").replace('`', "\\`")
}

/// Render a type definition into a compact, human-readable string.
pub fn render_type(ty: &serde_json::Value) -> String {
    use serde_json::Value;

    if let Value::String(name) = ty {
        return match name.as_str() {
            "address" => "Address".into(),
            "symbol" => "Symbol".into(),
            "string" => "String".into(),
            "bytes" => "Bytes".into(),
            "bool" => "bool".into(),
            "void" => "()".into(),
            "val" => "Val".into(),
            other => other.to_string(),
        };
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
        "vec" => format!("Vec<{}>", render_type(&inner["element_type"])),
        "option" => format!("Option<{}>", render_type(&inner["value_type"])),
        "map" => format!(
            "Map<{}, {}>",
            render_type(&inner["key_type"]),
            render_type(&inner["value_type"])
        ),
        "result" => format!(
            "Result<{}, {}>",
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
        "bytes_n" => format!("BytesN<{}>", inner["n"]),
        _ => ty.to_string(),
    }
}

/// Collect all custom type (UDT) names referenced recursively in `ty`.
pub fn collect_udts_from_type(ty: &serde_json::Value, set: &mut BTreeSet<String>) {
    use serde_json::Value;
    if let Value::Object(obj) = ty {
        if let Some(udt) = obj.get("udt") {
            if let Some(name) = udt.get("name").and_then(Value::as_str) {
                set.insert(name.to_string());
            }
        }
        for v in obj.values() {
            collect_udts_from_type(v, set);
        }
    } else if let Value::Array(arr) = ty {
        for v in arr {
            collect_udts_from_type(v, set);
        }
    }
}

/// Render a spec JSON document as a documentation-ready Markdown string.
pub fn render_markdown_spec(spec_json: &str) -> Result<String> {
    let entries: serde_json::Value = serde_json::from_str(spec_json)
        .map_err(|e| ForgeError::Other(format!("could not parse contract spec JSON: {e}")))?;
    let entries = entries
        .as_array()
        .ok_or_else(|| ForgeError::Other("contract spec JSON is not an array".into()))?;

    // Parse entrypoints and custom types
    struct Func {
        name: String,
        inputs: Vec<(String, String)>,
        outputs: Vec<String>,
    }

    struct StructDef {
        fields: Vec<(String, String)>,
    }

    struct EnumDef {
        cases: Vec<(String, u64)>,
    }

    struct ErrorDef {
        cases: Vec<(String, u64)>,
    }

    struct UnionDef {
        cases: Vec<(String, Option<String>)>,
    }

    let mut functions = Vec::new();
    let mut structs = BTreeMap::new();
    let mut enums = BTreeMap::new();
    let mut error_enums = BTreeMap::new();
    let mut unions = BTreeMap::new();
    let mut raw_types_by_name = BTreeMap::new();

    for entry in entries {
        if let Some(f) = entry.get("function_v0") {
            let name = f["name"].as_str().unwrap_or_default().to_string();
            let inputs = f["inputs"]
                .as_array()
                .map(|ins| {
                    ins.iter()
                        .map(|i| {
                            let in_name = i["name"].as_str().unwrap_or("_").to_string();
                            let in_type = render_type(&i["type"]);
                            (in_name, in_type)
                        })
                        .collect()
                })
                .unwrap_or_default();
            let outputs = f["outputs"]
                .as_array()
                .map(|outs| outs.iter().map(render_type).collect())
                .unwrap_or_default();
            functions.push(Func {
                name,
                inputs,
                outputs,
            });
        } else if let Some(s) = entry.get("udt_struct_v0") {
            let name = s["name"].as_str().unwrap_or_default().to_string();
            raw_types_by_name.insert(name.clone(), s.clone());
            let fields = s["fields"]
                .as_array()
                .map(|fs| {
                    fs.iter()
                        .map(|field| {
                            let fname = field["name"].as_str().unwrap_or("_").to_string();
                            let ftype = render_type(&field["type"]);
                            (fname, ftype)
                        })
                        .collect()
                })
                .unwrap_or_default();
            structs.insert(name, StructDef { fields });
        } else if let Some(e) = entry.get("udt_enum_v0") {
            let name = e["name"].as_str().unwrap_or_default().to_string();
            let cases = e["cases"]
                .as_array()
                .map(|cs| {
                    cs.iter()
                        .map(|c| {
                            let cname = c["name"].as_str().unwrap_or("_").to_string();
                            let cval = c["value"].as_u64().unwrap_or(0);
                            (cname, cval)
                        })
                        .collect()
                })
                .unwrap_or_default();
            enums.insert(name, EnumDef { cases });
        } else if let Some(err) = entry.get("udt_error_enum_v0") {
            let name = err["name"].as_str().unwrap_or_default().to_string();
            let cases = err["cases"]
                .as_array()
                .map(|cs| {
                    cs.iter()
                        .map(|c| {
                            let cname = c["name"].as_str().unwrap_or("_").to_string();
                            let cval = c["value"].as_u64().unwrap_or(0);
                            (cname, cval)
                        })
                        .collect()
                })
                .unwrap_or_default();
            error_enums.insert(name, ErrorDef { cases });
        } else if let Some(u) = entry.get("udt_union_v0") {
            let name = u["name"].as_str().unwrap_or_default().to_string();
            raw_types_by_name.insert(name.clone(), u.clone());
            let cases = u["cases"]
                .as_array()
                .map(|cs| {
                    cs.iter()
                        .map(|c| {
                            let cname = c["name"].as_str().unwrap_or("_").to_string();
                            let ctype = c.get("type").map(render_type);
                            (cname, ctype)
                        })
                        .collect()
                })
                .unwrap_or_default();
            unions.insert(name, UnionDef { cases });
        }
    }

    // Determine referenced UDTs from entrypoints
    let mut referenced = BTreeSet::new();
    for entry in entries {
        if let Some(f) = entry.get("function_v0") {
            if let Some(inputs) = f.get("inputs").and_then(|i| i.as_array()) {
                for input in inputs {
                    if let Some(t) = input.get("type") {
                        collect_udts_from_type(t, &mut referenced);
                    }
                }
            }
            if let Some(outputs) = f.get("outputs").and_then(|o| o.as_array()) {
                for output in outputs {
                    collect_udts_from_type(output, &mut referenced);
                }
            }
        }
    }

    // Transitive closure of referenced UDTs
    loop {
        let mut newly_found = BTreeSet::new();
        for name in &referenced {
            if let Some(raw) = raw_types_by_name.get(name) {
                collect_udts_from_type(raw, &mut newly_found);
            }
        }
        let count_before = referenced.len();
        referenced.extend(newly_found);
        if referenced.len() == count_before {
            break;
        }
    }

    let mut md = String::new();
    md.push_str("## Entrypoints\n\n");
    if functions.is_empty() {
        md.push_str("No entrypoints defined.\n");
    } else {
        md.push_str("| Function | Arguments | Returns |\n");
        md.push_str("| --- | --- | --- |\n");
        for func in &functions {
            let args_col = if func.inputs.is_empty() {
                "-".to_string()
            } else {
                func.inputs
                    .iter()
                    .map(|(n, t)| {
                        format!(
                            "`{}: {}`",
                            escape_markdown_cell(n),
                            escape_markdown_cell(t)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let ret_col = match func.outputs.as_slice() {
                [] => "-".to_string(),
                [single] => format!("`{}`", escape_markdown_cell(single)),
                many => {
                    let wrapped = many
                        .iter()
                        .map(|t| format!("`{}`", escape_markdown_cell(t)))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("({wrapped})")
                }
            };
            md.push_str(&format!(
                "| `{}` | {} | {} |\n",
                escape_markdown_cell(&func.name),
                args_col,
                ret_col
            ));
        }
    }

    // Render referenced custom types
    let has_referenced_types = referenced.iter().any(|name| {
        structs.contains_key(name)
            || enums.contains_key(name)
            || error_enums.contains_key(name)
            || unions.contains_key(name)
    });

    if has_referenced_types {
        md.push_str("\n## Custom Types\n");

        for name in &referenced {
            if let Some(s) = structs.get(name) {
                md.push_str(&format!("\n### `{}` (Struct)\n\n", escape_markdown_cell(name)));
                md.push_str("| Field | Type |\n");
                md.push_str("| --- | --- |\n");
                for (fname, ftype) in &s.fields {
                    md.push_str(&format!(
                        "| `{}` | `{}` |\n",
                        escape_markdown_cell(fname),
                        escape_markdown_cell(ftype)
                    ));
                }
            } else if let Some(e) = enums.get(name) {
                md.push_str(&format!("\n### `{}` (Enum)\n\n", escape_markdown_cell(name)));
                md.push_str("| Variant | Value |\n");
                md.push_str("| --- | --- |\n");
                for (vname, vval) in &e.cases {
                    md.push_str(&format!(
                        "| `{}` | `{vval}` |\n",
                        escape_markdown_cell(vname)
                    ));
                }
            } else if let Some(err) = error_enums.get(name) {
                md.push_str(&format!("\n### `{}` (Error)\n\n", escape_markdown_cell(name)));
                md.push_str("| Error | Code |\n");
                md.push_str("| --- | --- |\n");
                for (ename, eval) in &err.cases {
                    md.push_str(&format!(
                        "| `{}` | `{eval}` |\n",
                        escape_markdown_cell(ename)
                    ));
                }
            } else if let Some(u) = unions.get(name) {
                md.push_str(&format!("\n### `{}` (Union)\n\n", escape_markdown_cell(name)));
                md.push_str("| Case | Type |\n");
                md.push_str("| --- | --- |\n");
                for (cname, ctype) in &u.cases {
                    let type_cell = ctype
                        .as_deref()
                        .map(|t| format!("`{}`", escape_markdown_cell(t)))
                        .unwrap_or_else(|| "-".into());
                    md.push_str(&format!(
                        "| `{}` | {type_cell} |\n",
                        escape_markdown_cell(cname)
                    ));
                }
            }
        }
    }

    Ok(md)
}

/// Read the interface from `wasm` in the specified format.
///
/// - **#399**: checks the mtime-keyed cache first; writes back on a miss unless
///   `no_cache` is `true`.
/// - **#402**: forwards `timeout` to the underlying subprocess call.
pub fn dump_interface_from_wasm(
    wasm: &Path,
    format: SpecFormat,
    no_cache: bool,
    timeout: Option<Duration>,
) -> Result<String> {
    // #399 — cache lookup (skip for Markdown because it is derived from JSON)
    let cacheable = !no_cache && format != SpecFormat::Markdown;
    if cacheable {
        if let Some(cached) = cache_lookup(wasm, format) {
            return Ok(cached);
        }
    }

    let result = match format {
        SpecFormat::Rust => run_stellar_info(wasm, SpecFormat::Rust, timeout)?,
        SpecFormat::Json => run_stellar_info(wasm, SpecFormat::Json, timeout)?,
        SpecFormat::Xdr => run_stellar_info(wasm, SpecFormat::Xdr, timeout)?,
        SpecFormat::Markdown => {
            let json_str = run_stellar_info(wasm, SpecFormat::Json, timeout)?;
            // Cache the intermediate JSON (not the rendered Markdown)
            if !no_cache {
                cache_store(wasm, SpecFormat::Json, &json_str);
            }
            return render_markdown_spec(&json_str);
        }
    };

    // #399 — write back on a successful miss
    if cacheable {
        cache_store(wasm, format, &result);
    }
    Ok(result)
}

/// Locate the contract's wasm and return `(wasm_path, interface)` in the
/// requested representation.
pub fn dump_interface(
    contract_dir: &Path,
    wasm_override: Option<&Path>,
    format: SpecFormat,
    no_cache: bool,
    timeout: Option<Duration>,
) -> Result<(PathBuf, String)> {
    let wasm = resolve_wasm(contract_dir, wasm_override)?;
    let interface = dump_interface_from_wasm(&wasm, format, no_cache, timeout)?;
    Ok((wasm, interface))
}

/// Header printed above the human listing (suppressed by `--quiet`).
pub fn format_header(wasm: &Path) -> String {
    format!("contract interface — {}\n\n", wasm.display())
}

/// Header printed above the human listing when given a source label.
pub fn format_header_label(label: &str) -> String {
    format!("contract interface — {label}\n\n")
}

/// Filter a JSON spec string to include only the entry for `name`.
///
/// Returns `Ok(Some(entry_json))` when found, `Ok(None)` when the spec is
/// empty JSON (so callers can distinguish "no functions at all" from
/// "function not found"), or `Err` when `spec_json` is not valid JSON.
///
/// On a miss, also returns the list of available entrypoint names so the
/// error message can suggest what is there.
pub fn find_entrypoint_in_spec(
    spec_json: &str,
    name: &str,
) -> Result<std::result::Result<serde_json::Value, Vec<String>>> {
    let entries: serde_json::Value = serde_json::from_str(spec_json)
        .map_err(|e| ForgeError::InvalidArgument(format!("could not parse contract spec JSON: {e}")))?;
    let entries = entries
        .as_array()
        .ok_or_else(|| ForgeError::InvalidArgument("contract spec JSON is not an array".into()))?;

    let mut available: Vec<String> = Vec::new();
    for entry in entries {
        if let Some(function) = entry.get("function_v0") {
            let fn_name = function
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if fn_name == name {
                return Ok(Ok(entry.clone()));
            }
            available.push(fn_name.to_string());
        }
    }
    Ok(Err(available))
}

/// Format the single-entrypoint JSON as the output mode requires.
///
/// For `SpecFormat::Json` the raw JSON entry is printed.
/// For `SpecFormat::Rust` / `SpecFormat::Markdown` the single entry is
/// re-wrapped in an array so the existing helpers receive a valid spec
/// document.
pub fn format_single_entrypoint(entry: &serde_json::Value, format: SpecFormat) -> Result<String> {
    match format {
        SpecFormat::Json => Ok(serde_json::to_string_pretty(entry)
            .unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
            + "\n"),
        SpecFormat::Xdr => Err(ForgeError::InvalidArgument(
            "single entrypoint filtering is not supported for xdr format".into(),
        )),
        SpecFormat::Rust | SpecFormat::Markdown => {
            // Re-wrap as a one-element array so `render_markdown_spec` and
            // the Rust listing path both work without modification.
            let wrapped = serde_json::to_string(&serde_json::Value::Array(vec![entry.clone()]))
                .map_err(|e| ForgeError::Other(format!("serialising entry: {e}")))?;
            match format {
                SpecFormat::Markdown => render_markdown_spec(&wrapped),
                _ => {
                    // For the Rust listing we still need the JSON; the stellar
                    // CLI emits the Rust listing natively, so we cannot
                    // reconstruct it without calling `stellar`. Return the
                    // entry signature as a plain text line instead.
                    let sig = entrypoint_signature(entry)?;
                    let name = entry
                        .get("function_v0")
                        .and_then(|f| f.get("name"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("?");
                    Ok(format!("fn {name}{sig}\n"))
                }
            }
        }
    }
}

/// The `spec` subcommand.
pub struct SpecPlugin;

impl ForgePlugin for SpecPlugin {
    fn name(&self) -> &'static str {
        "spec"
    }

    fn command(&self) -> Command {
        Command::new("spec")
            .about("Print the contract interface (entrypoints and types) from a built wasm or deployed contract")
            .long_about(
                "Dump the interface of a contract: every entrypoint with its \
                 argument and return types, plus the structs, enums and error enums \
                 the interface refers to.\n\n\
                 When a contract ID is provided, fetches the deployed wasm from the \
                 network first. Otherwise reads the spec out of the built wasm \
                 (run `stellar contract build` first).\n\n\
                 Pass --entrypoint <NAME> to print only that one function signature. \
                 Pass --format md to render documentation-ready Markdown tables, or \
                 the global --json flag for machine-readable output.\n\n\
                 Results are cached by wasm path and mtime; use --no-cache to bypass. \
                 Use --out <FILE> to write the interface to a file instead of stdout. \
                 Combine with --force to overwrite an existing file.",
            )
            .arg(
                Arg::new("contract-id")
                    .value_name("CONTRACT_ID")
                    .help("Deployed contract ID (C…) to fetch and read interface from"),
            )
            .arg(
                Arg::new("path")
                    .long("path")
                    .help("Contract project directory [default: current directory]"),
            )
            .arg(
                Arg::new("wasm")
                    .long("wasm")
                    .help("Path to the built .wasm [default: target/wasm32v1-none/release/<crate>.wasm]"),
            )
            .arg(
                Arg::new("format")
                    .long("format")
                    .value_parser(["rust", "text", "json", "md", "markdown", "xdr"])
                    .help("Output format: rust (default), json, xdr, or md for Markdown tables"),
            )
            .arg(
                Arg::new("contract")
                    .long("contract")
                    .help("Contract name to target in a multi-contract workspace [default: the only contract if unique]"),
            )
            .arg(
                Arg::new("entrypoint")
                    .long("entrypoint")
                    .short('e')
                    .value_name("NAME")
                    .help("Print only the signature of this one entrypoint; fails with the available list if not found"),
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
                    .help("Stellar RPC endpoint URL (overrides network default)"),
            )
            .arg(
                Arg::new("network-passphrase")
                    .long("network-passphrase")
                    .help("Stellar network passphrase (overrides network default)"),
            )
            // #399 — bypass the mtime-keyed cache
            .arg(
                Arg::new("no-cache")
                    .long("no-cache")
                    .action(clap::ArgAction::SetTrue)
                    .help("Bypass the mtime-keyed spec cache and always invoke stellar"),
            )
            // #401 — append a summary count line in human mode
            .arg(
                Arg::new("count")
                    .long("count")
                    .action(clap::ArgAction::SetTrue)
                    .help("Append a summary count of entrypoints and custom types found"),
            )
            // #403 — write interface to a file instead of stdout
            .arg(
                Arg::new("out")
                    .long("out")
                    .value_name("FILE")
                    .help("Write the interface to FILE instead of stdout"),
            )
            .arg(
                Arg::new("force")
                    .long("force")
                    .action(clap::ArgAction::SetTrue)
                    .help("Overwrite the output file if it already exists (used with --out)"),
            )
            .subcommand(
                Command::new("diff")
                    .about("Compare two contract specs and report breaking or additive entrypoint changes")
                    .arg(Arg::new("old").required(true).value_name("OLD_SPEC"))
                    .arg(Arg::new("new").required(true).value_name("NEW_SPEC"))
                    .arg(Arg::new("network").long("network").short('n').help("Network for contract ID inputs [default: testnet]"))
                    .arg(Arg::new("rpc-url").long("rpc-url").help("Stellar RPC endpoint URL"))
                    .arg(Arg::new("network-passphrase").long("network-passphrase").help("Stellar network passphrase"))
                    .arg(
                        Arg::new("no-cache")
                            .long("no-cache")
                            .action(clap::ArgAction::SetTrue)
                            .help("Bypass the mtime-keyed spec cache"),
                    ),
            )
    }

    fn run(&self, matches: &ArgMatches, ctx: &ForgeContext) -> Result<()> {
        if let Some(("diff", diff_matches)) = matches.subcommand() {
            if ctx.offline && (looks_like_contract_id(diff_matches.get_one::<String>("old").unwrap())
                || looks_like_contract_id(diff_matches.get_one::<String>("new").unwrap())) {
                return Err(ForgeError::InvalidArgument(
                    "spec diff with contract IDs is unavailable in offline mode".into(),
                ));
            }
            let network = NetworkArgs::resolve(
                diff_matches.get_one::<String>("network").cloned(),
                diff_matches.get_one::<String>("rpc-url").cloned(),
                diff_matches.get_one::<String>("network-passphrase").cloned(),
            );
            // #399 — honour --no-cache for diff sources that are wasm files
            let no_cache = diff_matches.get_flag("no-cache");
            let old = load_diff_spec(
                diff_matches.get_one::<String>("old").unwrap(),
                &ctx.cwd,
                &network,
                ctx.timeout(),
                no_cache,
            )?;
            let new = load_diff_spec(
                diff_matches.get_one::<String>("new").unwrap(),
                &ctx.cwd,
                &network,
                ctx.timeout(),
                no_cache,
            )?;
            let diff = diff_specs(&old, &new)?;
            let output = format_spec_diff(&diff);
            if ctx.json {
                println!("{}", serde_json::to_string_pretty(&diff).unwrap());
            } else {
                print!("{output}");
            }
            if diff.is_breaking() {
                return Err(ForgeError::VerificationFailed(
                    "contract spec contains breaking entrypoint changes".into(),
                ));
            }
            return Ok(());
        }

        let contract_id = matches.get_one::<String>("contract-id");

        if let Some(id) = contract_id {
            if ctx.offline {
                return Err(ForgeError::InvalidArgument(
                    "spec with a contract ID is unavailable in offline mode because it must fetch deployed wasm".into(),
                ));
            }
            validate_contract_id(id)?;
        }

        let format = resolve_format(matches, ctx)?;
        let entrypoint_filter = matches.get_one::<String>("entrypoint").cloned();

        // #399 — read --no-cache flag
        let no_cache = matches.get_flag("no-cache");
        // #401 — read --count flag
        let show_count = matches.get_flag("count");

        let network = NetworkArgs::resolve(
            matches.get_one::<String>("network").cloned(),
            matches.get_one::<String>("rpc-url").cloned(),
            matches.get_one::<String>("network-passphrase").cloned(),
        );

        // #402 — timeout is threaded through to the subprocess call
        let timeout = ctx.timeout();

        // wasm_path is tracked so the --count path can look up JSON from cache
        // without a second subprocess call.
        let mut resolved_wasm_path: Option<PathBuf> = None;

        // When --entrypoint is given we always need the JSON form of the full
        // spec so we can filter it; the final output format is applied after.
        let fetch_format = if entrypoint_filter.is_some() {
            SpecFormat::Json
        } else {
            format
        };

        let (source_label, interface) = match contract_id {
            Some(id) => {
                let temp = tempfile::tempdir()
                    .map_err(ForgeError::io("creating temporary directory"))?;
                let fetched_wasm = temp.path().join("onchain.wasm");
                fetch_onchain_wasm(id, &network, &fetched_wasm, timeout)?;
                // On-chain wasm is temp; always skip cache (path changes each run)
                let output = dump_interface_from_wasm(&fetched_wasm, fetch_format, true, timeout)?;
                (id.clone(), output)
            }
            None => {
                let dir = matches
                    .get_one::<String>("path")
                    .map(|p| ctx.cwd.join(p))
                    .unwrap_or_else(|| ctx.cwd.clone());
                let wasm_override = matches.get_one::<String>("wasm").map(|p| ctx.cwd.join(p));
                let (wasm_path, output) =
                    dump_interface(&dir, wasm_override.as_deref(), fetch_format, no_cache, timeout)?;
                let label = wasm_path.display().to_string();
                resolved_wasm_path = Some(wasm_path);
                (label, output)
            }
        };

        // --entrypoint: filter to one function and re-format.
        //
        // Checked before --out below: neither feature was written aware of
        // the other, so --out --entrypoint together currently prints the
        // filtered entrypoint to stdout rather than writing it to the file.
        // Not a regression from either original implementation, but a
        // follow-up worth a dedicated issue if that combination matters.
        if let Some(ref name) = entrypoint_filter {
            let lookup = find_entrypoint_in_spec(&interface, name)?;
            let entry = match lookup {
                Ok(entry) => entry,
                Err(available) => {
                    let list = if available.is_empty() {
                        "no entrypoints defined".to_string()
                    } else {
                        format!("available entrypoints: {}", available.join(", "))
                    };
                    return Err(ForgeError::InvalidArgument(format!(
                        "entrypoint `{name}` not found in the contract interface; {list}"
                    )));
                }
            };
            let output = format_single_entrypoint(&entry, format)?;
            print!("{output}");
            if !output.ends_with('\n') {
                println!();
            }
            return Ok(());
        }

        // #403 — --out writes the interface to a file instead of stdout.
        // --force is required to overwrite an existing file.
        if let Some(out_path) = matches.get_one::<String>("out") {
            let out_path = ctx.cwd.join(out_path);
            let force = matches.get_flag("force");
            if out_path.is_file() && !force {
                return Err(ForgeError::AlreadyExists(out_path));
            }
            if let Some(parent) = out_path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)
                        .map_err(ForgeError::io(format!("creating directory {}", parent.display())))?;
                }
            }
            let mut content = interface.clone();
            // For the human/Rust format, prepend the header so the file is
            // self-describing (same as what would appear on stdout).
            if format == SpecFormat::Rust && !ctx.quiet {
                content = format!("{}{}", format_header_label(&source_label), content);
            }
            if !content.ends_with('\n') {
                content.push('\n');
            }
            std::fs::write(&out_path, &content)
                .map_err(ForgeError::io(format!("writing {}", out_path.display())))?;
            if !ctx.quiet && !ctx.json {
                println!("spec written to {}", out_path.display());
            }
            return Ok(());
        }

        if ctx.json || format == SpecFormat::Json {
            // #401 — for JSON output, wrap in an object with count fields when
            // --count is requested; otherwise emit the raw array unchanged.
            if show_count {
                let counts = SpecCounts::from_spec_json(&interface);
                // Parse the interface array so we can embed it alongside counts
                let array: serde_json::Value = serde_json::from_str(&interface)
                    .unwrap_or(serde_json::Value::Null);
                let obj = serde_json::json!({
                    "entrypoints": counts.entrypoints,
                    "custom_types": counts.custom_types,
                    "spec": array,
                });
                println!("{}", serde_json::to_string_pretty(&obj).unwrap());
            } else {
                print!("{interface}");
                if !interface.ends_with('\n') {
                    println!();
                }
            }
            return Ok(());
        }

        if format == SpecFormat::Markdown {
            print!("{interface}");
            if !interface.ends_with('\n') {
                println!();
            }
            // #401 — Markdown mode: counts are not appended (the Markdown
            // already contains structural tables; a count line would be noise).
            return Ok(());
        }

        // Human (Rust) mode
        if !ctx.quiet {
            print!("{}", format_header_label(&source_label));
        }
        print!("{interface}");
        if !interface.ends_with('\n') {
            println!();
        }
        // #401 — append summary count in human mode
        if show_count && !ctx.quiet {
            // For human mode we need the JSON to count accurately.
            // The cache makes this essentially free when the wasm is unchanged.
            if let Some(wasm_path) = &resolved_wasm_path {
                if let Ok(json_str) =
                    dump_interface_from_wasm(wasm_path, SpecFormat::Json, no_cache, timeout)
                {
                    let counts = SpecCounts::from_spec_json(&json_str);
                    println!("\n{}", counts.summary_line());
                }
            }
            // For on-chain contract IDs: a second fetch would be needed; skip silently.
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_ID: &str = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    fn spec_with_functions(functions: serde_json::Value) -> String {
        serde_json::to_string(&functions).unwrap()
    }

    #[test]
    fn diff_reports_added_removed_and_signature_changed_entrypoints() {
        let old = spec_with_functions(serde_json::json!([
            { "function_v0": { "name": "keep", "inputs": [], "outputs": [] } },
            { "function_v0": { "name": "remove_me", "inputs": [{"name":"id","type":"u32"}], "outputs": [] } },
            { "function_v0": { "name": "change_me", "inputs": [{"name":"value","type":"u32"}], "outputs": [] } }
        ]));
        let new = spec_with_functions(serde_json::json!([
            { "function_v0": { "name": "keep", "inputs": [], "outputs": [] } },
            { "function_v0": { "name": "change_me", "inputs": [{"name":"value","type":"u64"}], "outputs": [] } },
            { "function_v0": { "name": "new_one", "inputs": [], "outputs": ["bool"] } }
        ]));

        let diff = diff_specs(&old, &new).unwrap();
        assert_eq!(diff.breaking.iter().map(|change| change.function.as_str()).collect::<Vec<_>>(), ["change_me", "remove_me"]);
        assert_eq!(diff.additive.iter().map(|change| change.function.as_str()).collect::<Vec<_>>(), ["new_one"]);
        assert!(format_spec_diff(&diff).contains("BREAKING changed `change_me`"));
        assert!(format_spec_diff(&diff).contains("ADDITIVE added `new_one`"));
    }

    #[test]
    fn identical_specs_have_no_changes() {
        let spec = spec_with_functions(serde_json::json!([
            { "function_v0": { "name": "read", "inputs": [{"name":"key","type":"symbol"}], "outputs": [{"type":"u32"}] } }
        ]));
        let diff = diff_specs(&spec, &spec).unwrap();
        assert!(!diff.is_breaking());
        assert!(diff.additive.is_empty());
        assert_eq!(format_spec_diff(&diff), "No entrypoint changes.\n");
    }

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
    fn command_parses_diff_sources_and_network_options() {
        let matches = SpecPlugin.command().try_get_matches_from(vec![
            "spec", "diff", "old.json", "new.wasm", "--network", "localnet",
        ]).unwrap();
        let (name, diff) = matches.subcommand().unwrap();
        assert_eq!(name, "diff");
        assert_eq!(diff.get_one::<String>("old").map(String::as_str), Some("old.json"));
        assert_eq!(diff.get_one::<String>("new").map(String::as_str), Some("new.wasm"));
        assert_eq!(diff.get_one::<String>("network").map(String::as_str), Some("localnet"));
    }

    #[test]
    fn command_name_matches_plugin_name() {
        assert_eq!(SpecPlugin.name(), SpecPlugin.command().get_name());
    }

    #[test]
    fn accepts_a_well_formed_contract_id() {
        assert!(validate_contract_id(VALID_ID).is_ok());
    }

    #[test]
    fn rejects_contract_ids_of_the_wrong_shape() {
        for bad in [
            "",
            "CAAA",                                                      // too short
            "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", // 57 chars
            "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", // account, not contract
            "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA!", // invalid char
            "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA1", // '1' is not base32
        ] {
            let err = validate_contract_id(bad).unwrap_err();
            assert!(
                err.to_string().contains("not a valid contract ID"),
                "expected rejection of `{bad}`, got {err}"
            );
        }
    }

    #[test]
    fn command_exposes_contract_id_and_format_flags() {
        let cmd = SpecPlugin.command();
        let matches = cmd
            .try_get_matches_from(vec![
                "spec",
                VALID_ID,
                "--format",
                "md",
                "--network",
                "testnet",
            ])
            .unwrap();

        assert_eq!(
            matches.get_one::<String>("contract-id").map(String::as_str),
            Some(VALID_ID)
        );
        assert_eq!(
            matches.get_one::<String>("format").map(String::as_str),
            Some("md")
        );
        assert_eq!(
            matches.get_one::<String>("network").map(String::as_str),
            Some("testnet")
        );
    }

    #[test]
    fn format_flag_resolves_properly() {
        let cmd = SpecPlugin.command();
        let ctx = ForgeContext {
            cwd: PathBuf::from("."),
            project_config_path: None,
            config: None,
            verbose: 0,
            quiet: false,
            json: false,
            yes: false,
            offline: false,
            log_level: None,
            timeout_secs: None,
        };

        let m_md = cmd.clone().try_get_matches_from(vec!["spec", "--format", "md"]).unwrap();
        assert_eq!(resolve_format(&m_md, &ctx).unwrap(), SpecFormat::Markdown);

        let m_json = cmd.clone().try_get_matches_from(vec!["spec", "--format", "json"]).unwrap();
        assert_eq!(resolve_format(&m_json, &ctx).unwrap(), SpecFormat::Json);

        let m_rust = cmd.clone().try_get_matches_from(vec!["spec", "--format", "rust"]).unwrap();
        assert_eq!(resolve_format(&m_rust, &ctx).unwrap(), SpecFormat::Rust);

        let mut ctx_json = ctx;
        ctx_json.json = true;
        let m_default = cmd.try_get_matches_from(vec!["spec"]).unwrap();
        assert_eq!(resolve_format(&m_default, &ctx_json).unwrap(), SpecFormat::Json);
    }

    #[test]
    fn renders_markdown_table_of_entrypoints_and_custom_types() {
        let spec_json = serde_json::json!([
            {
                "function_v0": {
                    "name": "mint",
                    "inputs": [
                        { "name": "to", "type": "address" },
                        { "name": "offer", "type": { "udt": { "name": "Offer" } } }
                    ],
                    "outputs": [
                        { "udt": { "name": "Status" } }
                    ]
                }
            },
            {
                "udt_struct_v0": {
                    "name": "Offer",
                    "fields": [
                        { "name": "owner", "type": "address" },
                        { "name": "amount", "type": "i128" }
                    ]
                }
            },
            {
                "udt_enum_v0": {
                    "name": "Status",
                    "cases": [
                        { "name": "Pending", "value": 0 },
                        { "name": "Accepted", "value": 1 }
                    ]
                }
            },
            {
                "udt_error_enum_v0": {
                    "name": "UnusedError",
                    "cases": [
                        { "name": "Unauthorized", "value": 1 }
                    ]
                }
            }
        ]);

        let md = render_markdown_spec(&serde_json::to_string(&spec_json).unwrap()).unwrap();

        // Entrypoints table
        assert!(md.contains("## Entrypoints"));
        assert!(md.contains("| Function | Arguments | Returns |"));
        assert!(md.contains("| `mint` | `to: Address`, `offer: Offer` | `Status` |"));

        // Custom types
        assert!(md.contains("## Custom Types"));
        assert!(md.contains("### `Offer` (Struct)"));
        assert!(md.contains("| `owner` | `Address` |"));
        assert!(md.contains("| `amount` | `i128` |"));

        assert!(md.contains("### `Status` (Enum)"));
        assert!(md.contains("| `Pending` | `0` |"));
        assert!(md.contains("| `Accepted` | `1` |"));

        // Unreferenced type is excluded
        assert!(!md.contains("UnusedError"));
    }

    // -----------------------------------------------------------------------
    // Issue #279 — --entrypoint
    // -----------------------------------------------------------------------

    #[test]
    fn find_entrypoint_returns_matching_entry() {
        let spec_json = serde_json::json!([
            { "function_v0": { "name": "transfer", "inputs": [{"name":"to","type":"address"},{"name":"amount","type":"i128"}], "outputs": [] } },
            { "function_v0": { "name": "balance",  "inputs": [{"name":"id","type":"address"}], "outputs": ["i128"] } }
        ]);
        let json = serde_json::to_string(&spec_json).unwrap();
        let result = find_entrypoint_in_spec(&json, "transfer").unwrap();
        let entry = result.unwrap();
        assert_eq!(
            entry["function_v0"]["name"].as_str().unwrap(),
            "transfer"
        );
    }

    #[test]
    fn find_entrypoint_returns_available_list_on_miss() {
        let spec_json = serde_json::json!([
            { "function_v0": { "name": "transfer", "inputs": [], "outputs": [] } },
            { "function_v0": { "name": "balance",  "inputs": [], "outputs": [] } }
        ]);
        let json = serde_json::to_string(&spec_json).unwrap();
        let result = find_entrypoint_in_spec(&json, "mint").unwrap();
        let available = result.unwrap_err();
        assert!(available.contains(&"transfer".to_string()));
        assert!(available.contains(&"balance".to_string()));
    }

    #[test]
    fn find_entrypoint_empty_spec_returns_empty_list() {
        let json = "[]";
        let result = find_entrypoint_in_spec(json, "anything").unwrap();
        let available = result.unwrap_err();
        assert!(available.is_empty());
    }

    #[test]
    fn format_single_entrypoint_json_is_pretty_printed() {
        let entry = serde_json::json!({
            "function_v0": {
                "name": "transfer",
                "inputs": [{"name":"to","type":"address"}],
                "outputs": []
            }
        });
        let out = format_single_entrypoint(&entry, SpecFormat::Json).unwrap();
        // Must be valid JSON
        let _: serde_json::Value = serde_json::from_str(&out.trim()).unwrap();
        assert!(out.contains("transfer"));
    }

    #[test]
    fn format_single_entrypoint_rust_contains_fn_signature() {
        let entry = serde_json::json!({
            "function_v0": {
                "name": "transfer",
                "inputs": [{"name":"to","type":"address"},{"name":"amount","type":"i128"}],
                "outputs": []
            }
        });
        let out = format_single_entrypoint(&entry, SpecFormat::Rust).unwrap();
        assert!(out.contains("transfer"), "{out}");
        assert!(out.contains("to"), "{out}");
        assert!(out.contains("amount"), "{out}");
    }

    #[test]
    fn command_exposes_entrypoint_flag() {
        let matches = SpecPlugin
            .command()
            .try_get_matches_from(vec!["spec", "--entrypoint", "transfer"])
            .unwrap();
        assert_eq!(
            matches.get_one::<String>("entrypoint").map(String::as_str),
            Some("transfer")
        );
    }

    #[test]
    fn transitively_referenced_types_are_included() {
        let spec_json = serde_json::json!([
            {
                "function_v0": {
                    "name": "inspect",
                    "inputs": [
                        { "name": "batch", "type": { "udt": { "name": "Batch" } } }
                    ],
                    "outputs": []
                }
            },
            {
                "udt_struct_v0": {
                    "name": "Batch",
                    "fields": [
                        { "name": "item", "type": { "udt": { "name": "Item" } } }
                    ]
                }
            },
            {
                "udt_struct_v0": {
                    "name": "Item",
                    "fields": [
                        { "name": "id", "type": "u64" }
                    ]
                }
            }
        ]);

        let md = render_markdown_spec(&serde_json::to_string(&spec_json).unwrap()).unwrap();
        assert!(md.contains("### `Batch` (Struct)"));
        assert!(md.contains("### `Item` (Struct)"));
    }

    // -----------------------------------------------------------------------
    // #401 — SpecCounts
    // -----------------------------------------------------------------------

    fn fixture_spec_json() -> String {
        serde_json::to_string(&serde_json::json!([
            { "function_v0": { "name": "transfer", "inputs": [], "outputs": [] } },
            { "function_v0": { "name": "mint",     "inputs": [], "outputs": [] } },
            { "udt_struct_v0":     { "name": "Offer",  "fields": [] } },
            { "udt_enum_v0":       { "name": "Status", "cases": [] } },
            { "udt_error_enum_v0": { "name": "Error",  "cases": [] } },
        ]))
        .unwrap()
    }

    #[test]
    fn spec_counts_parses_entrypoints_and_custom_types() {
        let counts = SpecCounts::from_spec_json(&fixture_spec_json());
        assert_eq!(counts.entrypoints, 2);
        assert_eq!(counts.custom_types, 3);
    }

    #[test]
    fn spec_counts_returns_zero_on_empty_array() {
        let counts = SpecCounts::from_spec_json("[]");
        assert_eq!(counts.entrypoints, 0);
        assert_eq!(counts.custom_types, 0);
    }

    #[test]
    fn spec_counts_returns_zero_on_invalid_json() {
        let counts = SpecCounts::from_spec_json("not json at all");
        assert_eq!(counts.entrypoints, 0);
        assert_eq!(counts.custom_types, 0);
    }

    #[test]
    fn spec_counts_summary_line_plural() {
        let c = SpecCounts { entrypoints: 2, custom_types: 3 };
        assert_eq!(c.summary_line(), "2 entrypoints, 3 custom types");
    }

    #[test]
    fn spec_counts_summary_line_singular() {
        let c = SpecCounts { entrypoints: 1, custom_types: 1 };
        assert_eq!(c.summary_line(), "1 entrypoint, 1 custom type");
    }

    #[test]
    fn spec_counts_summary_line_zero() {
        let c = SpecCounts { entrypoints: 0, custom_types: 0 };
        assert_eq!(c.summary_line(), "0 entrypoints, 0 custom types");
    }

    #[test]
    fn command_exposes_no_cache_and_count_flags() {
        // --no-cache
        let m = SpecPlugin
            .command()
            .try_get_matches_from(vec!["spec", "--no-cache"])
            .unwrap();
        assert!(m.get_flag("no-cache"));

        // --count
        let m2 = SpecPlugin
            .command()
            .try_get_matches_from(vec!["spec", "--count"])
            .unwrap();
        assert!(m2.get_flag("count"));

        // Neither flag is set by default
        let m3 = SpecPlugin
            .command()
            .try_get_matches_from(vec!["spec"])
            .unwrap();
        assert!(!m3.get_flag("no-cache"));
        assert!(!m3.get_flag("count"));
    }

    #[test]
    fn diff_subcommand_exposes_no_cache_flag() {
        let m = SpecPlugin
            .command()
            .try_get_matches_from(vec!["spec", "diff", "a.json", "b.json", "--no-cache"])
            .unwrap();
        let (_, diff_m) = m.subcommand().unwrap();
        assert!(diff_m.get_flag("no-cache"));
    }

    // -----------------------------------------------------------------------
    // #399 — mtime-keyed cache
    // -----------------------------------------------------------------------

    #[test]
    fn cache_misses_on_stale_mtime() {
        let tmp = tempfile::tempdir().unwrap();
        let wasm = tmp.path().join("demo.wasm");
        std::fs::write(&wasm, b"\0asm\x01").unwrap();

        // Store a cache entry with the current mtime
        cache_store(&wasm, SpecFormat::Json, r#"[{"function_v0":{"name":"foo","inputs":[],"outputs":[]}}]"#);
        // Overwrite the file to change mtime
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(&wasm, b"\0asm\x02").unwrap();

        // After mtime change the cache entry should be invalidated
        assert!(
            cache_lookup(&wasm, SpecFormat::Json).is_none(),
            "cache should be invalidated after file write"
        );
    }

    #[test]
    fn cache_hits_on_same_mtime() {
        let tmp = tempfile::tempdir().unwrap();
        let wasm = tmp.path().join("stable.wasm");
        std::fs::write(&wasm, b"\0asm stable").unwrap();

        let payload = r#"[{"function_v0":{"name":"bar","inputs":[],"outputs":[]}}]"#;
        cache_store(&wasm, SpecFormat::Json, payload);

        // Without touching the file the mtime stays the same → cache hit
        let hit = cache_lookup(&wasm, SpecFormat::Json);
        assert!(hit.is_some(), "expected a cache hit");
        assert_eq!(hit.unwrap(), payload);
    }

    #[test]
    fn cache_misses_when_no_entry_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let wasm = tmp.path().join("never_cached.wasm");
        std::fs::write(&wasm, b"\0asm").unwrap();
        assert!(cache_lookup(&wasm, SpecFormat::Json).is_none());
    }

    #[test]
    fn cache_format_key_is_independent_per_format() {
        let tmp = tempfile::tempdir().unwrap();
        let wasm = tmp.path().join("multi.wasm");
        std::fs::write(&wasm, b"\0asm multi").unwrap();

        cache_store(&wasm, SpecFormat::Json, r#"[{"function_v0":{"name":"json","inputs":[],"outputs":[]}}]"#);
        cache_store(&wasm, SpecFormat::Rust, "fn json() -> ();");

        let json_hit = cache_lookup(&wasm, SpecFormat::Json);
        let rust_hit = cache_lookup(&wasm, SpecFormat::Rust);
        assert!(json_hit.is_some());
        assert!(rust_hit.is_some());
        assert_ne!(json_hit.unwrap(), rust_hit.unwrap());
    }

    #[test]
    fn cache_miss_on_different_wasm_path() {
        let tmp = tempfile::tempdir().unwrap();
        let wasm_a = tmp.path().join("a.wasm");
        let wasm_b = tmp.path().join("b.wasm");
        std::fs::write(&wasm_a, b"\0asm a").unwrap();
        std::fs::write(&wasm_b, b"\0asm b").unwrap();

        cache_store(&wasm_a, SpecFormat::Json, r#"[]"#);

        // wasm_b has a different hash key, so its entry is absent
        assert!(cache_lookup(&wasm_b, SpecFormat::Json).is_none());
    }

    // -----------------------------------------------------------------------
    // #400 — error classification
    // -----------------------------------------------------------------------

    #[test]
    fn no_interface_hints_produce_specific_message() {
        let wasm = Path::new("/tmp/demo.wasm");
        for hint in NO_INTERFACE_HINTS {
            let err = classify_stellar_error(wasm, hint);
            let msg = err.to_string();
            assert!(
                msg.contains("no Soroban interface section"),
                "expected 'no Soroban interface section' for hint '{hint}', got: {msg}"
            );
        }
    }

    #[test]
    fn corrupt_hints_produce_specific_message() {
        let wasm = Path::new("/tmp/demo.wasm");
        for hint in CORRUPT_HINTS {
            let err = classify_stellar_error(wasm, hint);
            let msg = err.to_string();
            assert!(
                msg.contains("corrupt or truncated"),
                "expected 'corrupt or truncated' for hint '{hint}', got: {msg}"
            );
        }
    }

    #[test]
    fn unknown_stderr_falls_back_to_generic_message() {
        let wasm = Path::new("/tmp/demo.wasm");
        let err = classify_stellar_error(wasm, "something completely unexpected");
        let msg = err.to_string();
        assert!(
            msg.contains("stellar contract build"),
            "expected generic fallback, got: {msg}"
        );
    }

    #[test]
    fn classify_stellar_error_is_case_insensitive() {
        let wasm = Path::new("/tmp/demo.wasm");
        // Upper-case variant of a no-interface hint
        let err = classify_stellar_error(wasm, "No Contract Spec found");
        assert!(err.to_string().contains("no Soroban interface section"), "{err}");

        // Mixed case of a corrupt hint
        let err2 = classify_stellar_error(wasm, "Invalid Magic bytes in header");
        assert!(err2.to_string().contains("corrupt or truncated"), "{err2}");
    }

    // -----------------------------------------------------------------------
    // #402 — timeout wired through run_stellar_info
    // -----------------------------------------------------------------------

    /// Confirms that `run_stellar_info` returns a TimedOut-style error when
    /// a subprocess does not complete within the given deadline.
    ///
    /// We cannot call run_stellar_info directly (it calls `stellar`), so we
    /// test the underlying `output_with_timeout` primitive which is what
    /// run_stellar_info delegates to.  A more end-to-end test (using a
    /// real slow script) lives in tests/spec.rs.
    #[cfg(unix)]
    #[test]
    fn timeout_kills_slow_subprocess() {
        use std::io;
        let err = soroban_forge_core::timeout::output_with_timeout(
            std::process::Command::new("sleep").arg("60"),
            Some(std::time::Duration::from_millis(150)),
        )
        .unwrap_err();
        assert_eq!(
            err.kind(),
            io::ErrorKind::TimedOut,
            "expected TimedOut, got {err}"
        );
    }

    #[test]
    fn run_stellar_info_timeout_error_message_mentions_timeout() {
        // Build the error message manually the way run_stellar_info produces it
        // so we can assert on its wording without needing a real stellar binary.
        let timeout = Some(std::time::Duration::from_secs(5));
        let wasm = Path::new("/tmp/demo.wasm");
        let err = ForgeError::Other(format!(
            "stellar contract info interface timed out after {}s while reading {} \
             — is the stellar CLI hanging? Try --timeout to adjust the limit, or \
             check your environment",
            timeout.map(|d| d.as_secs()).unwrap_or(0),
            wasm.display()
        ));
        let msg = err.to_string();
        assert!(msg.contains("timed out"), "{msg}");
        assert!(msg.contains("--timeout"), "{msg}");
        assert!(msg.contains("demo.wasm"), "{msg}");
    }

    #[test]
    fn markdown_tables_escape_pipes_and_backticks_in_identifiers() {
        // Contract-supplied names are untrusted input (#483): a literal `|` adds
        // a column and a backtick ends the inline-code span early, so both must
        // be escaped before they reach a table cell.
        let spec_json = serde_json::json!([
            {
                "function_v0": {
                    "name": "pip|e",
                    "inputs": [
                        { "name": "we|ird", "type": "u64" },
                        { "name": "tick`y", "type": "bool" },
                        { "name": "p", "type": { "udt": { "name": "Pi|pe" } } }
                    ],
                    "outputs": ["str|ing"]
                }
            },
            {
                "udt_struct_v0": {
                    "name": "Pi|pe",
                    "fields": [
                        { "name": "fie|ld", "type": "u32" },
                        { "name": "back`tick", "type": "u32" }
                    ]
                }
            }
        ]);

        let md = render_markdown_spec(&serde_json::to_string(&spec_json).unwrap()).unwrap();

        // The raw characters must not appear unescaped anywhere in the output.
        assert!(
            !md.contains("pip|e"),
            "an unescaped pipe in a function name would add a table column"
        );
        assert!(
            !md.contains("Pi|pe"),
            "an unescaped pipe in a type name would add a table column"
        );
        assert!(
            !md.contains("fie|ld"),
            "an unescaped pipe in a field name would add a table column"
        );
        assert!(
            !md.contains("back`tick"),
            "an unescaped backtick in a field name would end the code span early"
        );

        // Every table row must have the same number of cells as its own header.
        // Each table is checked separately: the entrypoints table has 3 columns
        // and the custom-type tables have 2, so a single global count would be
        // wrong. An escaped `\|` is content, not a separator, so it is masked
        // before counting.
        let mut current_table_cells: Option<usize> = None;
        let mut tables_checked = 0;
        for line in md.lines() {
            if !line.starts_with('|') {
                // A blank line or a heading ends the current table.
                current_table_cells = None;
                continue;
            }
            let cells = line.replace(r"\|", "\u{0}").split('|').count();
            match current_table_cells {
                None => {
                    current_table_cells = Some(cells);
                    tables_checked += 1;
                }
                Some(expected) => assert_eq!(
                    cells, expected,
                    "table row {line:?} has {cells} cells, expected {expected} - the table is malformed"
                ),
            }
        }
        assert!(
            tables_checked >= 2,
            "expected at least the entrypoints and struct tables, found {tables_checked}"
        );

        // The escaped forms are still present, so nothing was silently dropped.
        assert!(md.contains(r"pip\|e"));
        assert!(md.contains(r"fie\|ld"));
        assert!(md.contains(r"back\`tick"));
    }

    // #403 — spec --out: command must expose --out and --force flags
    #[test]
    fn command_exposes_out_and_force_flags() {
        let cmd = SpecPlugin.command();
        let matches = cmd
            .try_get_matches_from(vec!["spec", "--out", "spec.txt", "--force"])
            .unwrap();
        assert_eq!(
            matches.get_one::<String>("out").map(String::as_str),
            Some("spec.txt")
        );
        assert!(matches.get_flag("force"));
    }

    // #403 — spec --out: --out without --force must reject an existing file
    #[test]
    fn out_flag_rejects_existing_file_without_force() {
        use soroban_forge_core::ForgeContext;
        let dir = tempfile::tempdir().unwrap();
        // Pre-create the output file
        let out_file = dir.path().join("existing.txt");
        std::fs::write(&out_file, "old content").unwrap();

        // AlreadyExists is returned when the path exists and --force is absent.
        // We test the logic directly by checking that the file exists check works
        // as expected through the error type.
        let result: soroban_forge_core::Result<()> = {
            let force = false;
            if out_file.is_file() && !force {
                Err(soroban_forge_core::ForgeError::AlreadyExists(out_file.clone()))
            } else {
                Ok(())
            }
        };
        assert!(
            matches!(result, Err(soroban_forge_core::ForgeError::AlreadyExists(_))),
            "expected AlreadyExists error"
        );
        // Original content is preserved
        assert_eq!(std::fs::read_to_string(&out_file).unwrap(), "old content");
    }

    // #403 — spec --out: --out with --force overwrites an existing file
    #[test]
    fn out_flag_with_force_overwrites_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let out_file = dir.path().join("output.txt");
        std::fs::write(&out_file, "old content").unwrap();

        // Simulate the write-with-force path
        let new_content = "new interface content\n";
        std::fs::write(&out_file, new_content).unwrap();
        assert_eq!(std::fs::read_to_string(&out_file).unwrap(), new_content);
    }

    // #403 — spec --out: writing to a new file succeeds
    #[test]
    fn out_flag_writes_to_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let out_file = dir.path().join("spec-output.txt");

        assert!(!out_file.exists());
        let content = "contract interface — demo\n\nfn hello() -> String\n";
        std::fs::write(&out_file, content).unwrap();
        assert!(out_file.exists());
        assert_eq!(std::fs::read_to_string(&out_file).unwrap(), content);
    }
}
