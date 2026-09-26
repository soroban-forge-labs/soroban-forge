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

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use clap::{Arg, ArgMatches, Command};
use serde::{Deserialize, Serialize};
use soroban_forge_core::{ForgeContext, ForgeError, ForgePlugin, Result};

/// Network used when neither `--network` nor `--rpc-url` is given.
pub const DEFAULT_NETWORK: &str = "testnet";

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
pub fn load_diff_spec(source: &str, cwd: &Path, network: &NetworkArgs) -> Result<String> {
    let path = cwd.join(source);
    if path.is_file() {
        if path.extension().and_then(|ext| ext.to_str()) == Some("wasm") {
            return dump_interface_from_wasm(&path, SpecFormat::Json);
        }
        return std::fs::read_to_string(&path)
            .map_err(ForgeError::io(format!("reading spec file {}", path.display())));
    }
    validate_contract_id(source)?;
    let temp = tempfile::tempdir().map_err(ForgeError::io("creating temporary directory"))?;
    let wasm = temp.path().join("contract.wasm");
    fetch_onchain_wasm(source, network, &wasm)?;
    dump_interface_from_wasm(&wasm, SpecFormat::Json)
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
fn fetch_onchain_wasm(contract_id: &str, network: &NetworkArgs, out_file: &Path) -> Result<()> {
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

    match cmd.output() {
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
}

impl SpecFormat {
    /// Value to pass to the CLI's `--output` flag.
    pub fn cli_output(self) -> &'static str {
        match self {
            SpecFormat::Rust => "rust",
            SpecFormat::Json | SpecFormat::Markdown => "json-formatted",
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
            other => Err(ForgeError::InvalidArgument(format!(
                "unsupported spec format `{other}`; expected `rust`, `json` or `md`"
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
                    .map(|(n, t)| format!("`{n}: {t}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let ret_col = match func.outputs.as_slice() {
                [] => "-".to_string(),
                [single] => format!("`{single}`"),
                many => {
                    let wrapped = many
                        .iter()
                        .map(|t| format!("`{t}`"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("({wrapped})")
                }
            };
            md.push_str(&format!("| `{}` | {} | {} |\n", func.name, args_col, ret_col));
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
                md.push_str(&format!("\n### `{name}` (Struct)\n\n"));
                md.push_str("| Field | Type |\n");
                md.push_str("| --- | --- |\n");
                for (fname, ftype) in &s.fields {
                    md.push_str(&format!("| `{fname}` | `{ftype}` |\n"));
                }
            } else if let Some(e) = enums.get(name) {
                md.push_str(&format!("\n### `{name}` (Enum)\n\n"));
                md.push_str("| Variant | Value |\n");
                md.push_str("| --- | --- |\n");
                for (vname, vval) in &e.cases {
                    md.push_str(&format!("| `{vname}` | `{vval}` |\n"));
                }
            } else if let Some(err) = error_enums.get(name) {
                md.push_str(&format!("\n### `{name}` (Error)\n\n"));
                md.push_str("| Error | Code |\n");
                md.push_str("| --- | --- |\n");
                for (ename, eval) in &err.cases {
                    md.push_str(&format!("| `{ename}` | `{eval}` |\n"));
                }
            } else if let Some(u) = unions.get(name) {
                md.push_str(&format!("\n### `{name}` (Union)\n\n"));
                md.push_str("| Case | Type |\n");
                md.push_str("| --- | --- |\n");
                for (cname, ctype) in &u.cases {
                    let type_cell = ctype.as_deref().map(|t| format!("`{t}`")).unwrap_or_else(|| "-".into());
                    md.push_str(&format!("| `{cname}` | {type_cell} |\n"));
                }
            }
        }
    }

    Ok(md)
}

/// Read the interface from `wasm` in the specified format.
pub fn dump_interface_from_wasm(wasm: &Path, format: SpecFormat) -> Result<String> {
    match format {
        SpecFormat::Rust => run_stellar_info(wasm, SpecFormat::Rust),
        SpecFormat::Json => run_stellar_info(wasm, SpecFormat::Json),
        SpecFormat::Markdown => {
            let json_str = run_stellar_info(wasm, SpecFormat::Json)?;
            render_markdown_spec(&json_str)
        }
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
    let interface = dump_interface_from_wasm(&wasm, format)?;
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
                 Pass --format md to render documentation-ready Markdown tables, or \
                 the global --json flag for machine-readable output.",
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
                    .value_parser(["rust", "text", "json", "md", "markdown"])
                    .help("Output format: rust (default), json, or md for Markdown tables"),
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
            .subcommand(
                Command::new("diff")
                    .about("Compare two contract specs and report breaking or additive entrypoint changes")
                    .arg(Arg::new("old").required(true).value_name("OLD_SPEC"))
                    .arg(Arg::new("new").required(true).value_name("NEW_SPEC"))
                    .arg(Arg::new("network").long("network").short('n').help("Network for contract ID inputs [default: testnet]"))
                    .arg(Arg::new("rpc-url").long("rpc-url").help("Stellar RPC endpoint URL"))
                    .arg(Arg::new("network-passphrase").long("network-passphrase").help("Stellar network passphrase")),
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
            let old = load_diff_spec(diff_matches.get_one::<String>("old").unwrap(), &ctx.cwd, &network)?;
            let new = load_diff_spec(diff_matches.get_one::<String>("new").unwrap(), &ctx.cwd, &network)?;
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

        let network = NetworkArgs::resolve(
            matches.get_one::<String>("network").cloned(),
            matches.get_one::<String>("rpc-url").cloned(),
            matches.get_one::<String>("network-passphrase").cloned(),
        );

        let (source_label, interface) = match contract_id {
            Some(id) => {
                let temp = tempfile::tempdir()
                    .map_err(ForgeError::io("creating temporary directory"))?;
                let fetched_wasm = temp.path().join("onchain.wasm");
                fetch_onchain_wasm(id, &network, &fetched_wasm)?;
                let output = dump_interface_from_wasm(&fetched_wasm, format)?;
                (id.clone(), output)
            }
            None => {
                let dir = matches
                    .get_one::<String>("path")
                    .map(|p| ctx.cwd.join(p))
                    .unwrap_or_else(|| ctx.cwd.clone());
                let wasm_override = matches.get_one::<String>("wasm").map(|p| ctx.cwd.join(p));
                let (wasm_path, output) = dump_interface(&dir, wasm_override.as_deref(), format)?;
                (wasm_path.display().to_string(), output)
            }
        };

        if ctx.json || format == SpecFormat::Json {
            print!("{interface}");
            if !interface.ends_with('\n') {
                println!();
            }
            return Ok(());
        }

        if format == SpecFormat::Markdown {
            print!("{interface}");
            if !interface.ends_with('\n') {
                println!();
            }
            return Ok(());
        }

        if !ctx.quiet {
            print!("{}", format_header_label(&source_label));
        }
        print!("{interface}");
        if !interface.ends_with('\n') {
            println!();
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
}
