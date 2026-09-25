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
//! `soroban-forge spec diff <old> <new>` compares two interfaces — each a
//! wasm file, a spec JSON file (as `spec --json` writes), or a deployed
//! contract ID — and classifies the differences as breaking (a removed
//! entrypoint, or one whose signature changed) or additive (a new
//! entrypoint), exiting `1` on a breaking change so CI can gate a release on
//! interface stability. See [`diff_specs`].

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use clap::{Arg, ArgMatches, Command};
use serde::Deserialize;
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

/// How to reach the network when fetching deployed contract wasm.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkArgs {
    pub network: Option<String>,
    pub rpc_url: Option<String>,
    pub network_passphrase: Option<String>,
}

impl NetworkArgs {
    /// `from_config`, when given, supplies defaults from `forge.toml`'s
    /// `[network]` table; CLI flags override file values.
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
    let out_str = out_file.to_str().ok_or_else(|| {
        ForgeError::Other(format!("path {} is not valid UTF-8", out_file.display()))
    })?;

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
            md.push_str(&format!(
                "| `{}` | {} | {} |\n",
                func.name, args_col, ret_col
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
                    let type_cell = ctype
                        .as_deref()
                        .map(|t| format!("`{t}`"))
                        .unwrap_or_else(|| "-".into());
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

// --- `spec diff`: interface stability check ---

/// Where a `spec diff` argument's interface comes from, auto-detected from
/// its shape: a `.json` file (as `spec --json` writes), a strkey contract ID
/// (`C` + 55 base32 characters, see [`validate_contract_id`]), or otherwise
/// a wasm file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecArg {
    File(PathBuf),
    Wasm(PathBuf),
    ContractId(String),
}

/// Classify a `spec diff` argument by its shape.
pub fn classify_spec_arg(arg: &str) -> SpecArg {
    if validate_contract_id(arg).is_ok() {
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

/// Resolve a `spec diff` argument to its interface JSON. A contract ID is
/// fetched the same way the top-level `spec <CONTRACT_ID>` does — via
/// `stellar contract fetch` into a temporary file, then the usual wasm
/// interface read — rather than a second, separate network code path.
pub fn read_spec_json(arg: &SpecArg, network: &NetworkArgs, offline: bool) -> Result<String> {
    match arg {
        SpecArg::File(path) => std::fs::read_to_string(path)
            .map_err(ForgeError::io(format!("reading {}", path.display()))),
        SpecArg::Wasm(path) => run_stellar_info(path, SpecFormat::Json),
        SpecArg::ContractId(id) => {
            if offline {
                return Err(ForgeError::InvalidArgument(format!(
                    "`{id}` looks like a contract ID, but spec diff is running with --offline — \
                     pass a wasm file or a spec JSON file instead, or drop --offline"
                )));
            }
            let temp =
                tempfile::tempdir().map_err(ForgeError::io("creating temporary directory"))?;
            let fetched_wasm = temp.path().join("onchain.wasm");
            fetch_onchain_wasm(id, network, &fetched_wasm)?;
            run_stellar_info(&fetched_wasm, SpecFormat::Json)
        }
    }
}

/// One entrypoint, as read from `stellar contract info interface --output json`.
struct Entrypoint {
    name: String,
    signature: String,
}

/// Render an `ScSpecTypeDef` from the CLI's JSON as a compact type string,
/// for entrypoint signature strings in a `spec diff` report.
///
/// Deliberately separate from [`render_type`]: that renderer's PascalCase,
/// Rust-flavoured style (`Vec<T>`, `Option<T>`) is for the `--format md`
/// documentation table; this one's lowercase style (`vec<T>`, `option<T>`)
/// only needs to be a stable, comparable string two specs can be diffed by.
fn render_signature_type(ty: &serde_json::Value) -> String {
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
        "vec" => format!("vec<{}>", render_signature_type(&inner["element_type"])),
        "option" => format!("option<{}>", render_signature_type(&inner["value_type"])),
        "map" => format!(
            "map<{}, {}>",
            render_signature_type(&inner["key_type"]),
            render_signature_type(&inner["value_type"])
        ),
        "result" => format!(
            "result<{}, {}>",
            render_signature_type(&inner["ok_type"]),
            render_signature_type(&inner["error_type"])
        ),
        "tuple" => {
            let types: Vec<String> = inner["value_types"]
                .as_array()
                .map(|types| types.iter().map(render_signature_type).collect())
                .unwrap_or_default();
            format!("({})", types.join(", "))
        }
        "bytes_n" => format!("bytes<{}>", inner["n"]),
        _ => ty.to_string(),
    }
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
                            render_signature_type(&input["type"])
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let outputs: Vec<String> = func["outputs"]
            .as_array()
            .map(|outputs| outputs.iter().map(render_signature_type).collect())
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

/// An entrypoint present on both sides whose signature differs.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ChangedEntrypoint {
    pub name: String,
    /// Signature in the old interface.
    pub old: String,
    /// Signature in the new interface.
    pub new: String,
}

/// Interface differences between an old and a new spec.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
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
            ctx.config.as_ref().map(|c| &c.network),
        );

        let (source_label, interface) = match contract_id {
            Some(id) => {
                let temp =
                    tempfile::tempdir().map_err(ForgeError::io("creating temporary directory"))?;
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

    const VALID_ID: &str = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

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
            "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",  // account, not contract
            "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA!",  // invalid char
            "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA1",  // '1' is not base32
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

        let m_md = cmd
            .clone()
            .try_get_matches_from(vec!["spec", "--format", "md"])
            .unwrap();
        assert_eq!(resolve_format(&m_md, &ctx).unwrap(), SpecFormat::Markdown);

        let m_json = cmd
            .clone()
            .try_get_matches_from(vec!["spec", "--format", "json"])
            .unwrap();
        assert_eq!(resolve_format(&m_json, &ctx).unwrap(), SpecFormat::Json);

        let m_rust = cmd
            .clone()
            .try_get_matches_from(vec!["spec", "--format", "rust"])
            .unwrap();
        assert_eq!(resolve_format(&m_rust, &ctx).unwrap(), SpecFormat::Rust);

        let mut ctx_json = ctx;
        ctx_json.json = true;
        let m_default = cmd.try_get_matches_from(vec!["spec"]).unwrap();
        assert_eq!(
            resolve_format(&m_default, &ctx_json).unwrap(),
            SpecFormat::Json
        );
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

    // --- spec diff: argument classification ---

    #[test]
    fn classifies_a_contract_id() {
        assert_eq!(
            classify_spec_arg(VALID_ID),
            SpecArg::ContractId(VALID_ID.to_string())
        );
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
        assert_eq!(
            classify_spec_arg("not-an-id"),
            SpecArg::Wasm(PathBuf::from("not-an-id"))
        );
    }

    #[test]
    fn offline_rejects_a_contract_id_before_touching_the_network() {
        let err = read_spec_json(
            &SpecArg::ContractId(VALID_ID.to_string()),
            &NetworkArgs::default(),
            true,
        )
        .unwrap_err();
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

    // --- spec diff: network args (config support) ---

    #[test]
    fn network_args_default_to_testnet() {
        let network = NetworkArgs::resolve(None, None, None, None);
        assert_eq!(network.cli_args(), vec!["--network", "testnet"]);
    }

    #[test]
    fn network_args_pull_defaults_from_config() {
        let cfg = soroban_forge_core::config::NetworkConfig {
            name: Some("futurenet".into()),
            rpc_url: None,
            passphrase: None,
        };
        let network = NetworkArgs::resolve(None, None, None, Some(&cfg));
        assert_eq!(network.cli_args(), vec!["--network", "futurenet"]);
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
    fn spec_json_funcs(funcs: &[FuncSpec]) -> String {
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
        let old = spec_json_funcs(&[
            ("balance", &[("id", json!("address"))], Some(json!("i128"))),
            (
                "burn",
                &[("from", json!("address")), ("amount", json!("i128"))],
                None,
            ),
            ("name", &[], Some(json!("string"))),
        ]);
        let new = spec_json_funcs(&[
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
        let spec = spec_json_funcs(&[("hello", &[("to", serde_json::json!("symbol"))], None)]);
        let diff = diff_specs(&spec, &spec).unwrap();
        assert!(diff.is_empty());
        assert!(!diff.is_breaking());
    }

    #[test]
    fn purely_additive_diff_is_not_breaking() {
        use serde_json::json;
        let old = spec_json_funcs(&[("balance", &[("id", json!("address"))], Some(json!("i128")))]);
        let new = spec_json_funcs(&[
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
        let old = spec_json_funcs(&[("burn", &[("from", json!("address"))], None)]);
        let new = spec_json_funcs(&[]);
        assert!(diff_specs(&old, &new).unwrap().is_breaking());
    }

    #[test]
    fn a_changed_signature_is_breaking() {
        use serde_json::json;
        let old = spec_json_funcs(&[("mint", &[("to", json!("address"))], None)]);
        let new = spec_json_funcs(&[(
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
    fn renders_compound_signature_types() {
        use serde_json::json;
        let ty = json!({"result": {
            "ok_type": {"vec": {"element_type": {"option": {"value_type": {"udt": {"name": "Offer"}}}}}},
            "error_type": {"udt": {"name": "Error"}}
        }});
        assert_eq!(
            render_signature_type(&ty),
            "result<vec<option<Offer>>, Error>"
        );
        assert_eq!(
            render_signature_type(
                &json!({"map": {"key_type": "symbol", "value_type": {"bytes_n": {"n": 32}}}})
            ),
            "map<symbol, bytes<32>>"
        );
        assert_eq!(
            render_signature_type(&json!({"tuple": {"value_types": []}})),
            "()"
        );
        assert_eq!(
            render_signature_type(&json!({"future": 1})),
            r#"{"future":1}"#
        );
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
        std::fs::write(&old_path, spec_json_funcs(&[("burn", &[], None)])).unwrap();
        std::fs::write(&new_path, spec_json_funcs(&[("mint", &[], None)])).unwrap();

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
