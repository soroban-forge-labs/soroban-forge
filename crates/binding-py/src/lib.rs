//! # soroban-forge-bindings-py
//!
//! `soroban-forge bindings-py` — generates a typed Python client (`client.py`)
//! from a built contract's wasm, mirroring `soroban-forge bindings ts`'s
//! public surface (`read_package_info`/`locate_wasm`/`generate_bindings`,
//! `--path`/`--wasm`/`--output`/`--force`, local and offline).
//!
//! ## Why this isn't a `stellar-cli` wrapper
//!
//! Every other soroban-forge module that touches a contract's interface
//! shells out to the official `stellar` CLI and never reimplements XDR/spec
//! decoding. That isn't possible here: `stellar contract bindings python`
//! is unimplemented — it prints a message pointing at a third-party tool,
//! [`stellar-contract-bindings`](https://github.com/lightsail-network/stellar-contract-bindings)
//! on PyPI, whose `python` command requires `--contract-id` and `--rpc-url`
//! and only works against an already-*deployed* contract. That is a
//! different model from every other soroban-forge subcommand (and from
//! `bindings ts`), which read a locally *built* wasm and need no network.
//!
//! So this module reads the interface the same official, first-party way
//! `spec`/`verify`/`bindings ts --react` do —
//! `stellar contract info interface --output json` — and renders the
//! Python source itself. The generated client's *runtime* (building,
//! simulating, signing and submitting transactions; encoding and decoding
//! `SCVal`s) is never reimplemented either: every generated method delegates
//! to the official `stellar-sdk` PyPI package's
//! `stellar_sdk.contract.ContractClient`/`AssembledTransaction` and its
//! `stellar_sdk.scval` conversion functions. What is generated is only the
//! thin, per-contract, per-method surface — the actual "bindings".
//!
//! ## Scope and known simplifications
//!
//! - `__constructor` is excluded, same as `bindings ts --react`'s hooks: a
//!   deployed contract cannot be re-constructed.
//! - `Result<T, E>` (spec `result`) renders as plain `T`. On the Soroban
//!   host, a contract function returning `Result<T, E>` traps on `Err`
//!   rather than returning a decodable error value, so `E` never appears on
//!   the wire for a successful call; the client-side error path is already
//!   an exception raised by `stellar_sdk` itself, not a value this code
//!   would decode.
//! - `muxed_address` is treated as `address` (`stellar_sdk.Address`, via
//!   `scval.from_address`/`to_address`): `stellar-sdk` 16.x has no
//!   dedicated muxed-address `scval` helper.
//! - A tagged enum's associated data is always rendered as a `values: tuple`
//!   field, whatever its arity — mirroring the TypeScript generator's
//!   `{tag, values}` shape — even though `stellar_sdk.scval.to_enum`/
//!   `from_enum` themselves are arity-sensitive (a bare `SCVal` for exactly
//!   one value, a `list[SCVal]` for two or more); see [`render_union`].

use std::path::{Path, PathBuf};

use clap::{Arg, ArgAction, ArgMatches, Command};
use serde::Deserialize;
use serde_json::Value;
use soroban_forge_core::{ForgeContext, ForgeError, ForgePlugin, Result};

const DEFAULT_OUTPUT_SUBDIR: &str = "bindings/python";

/// Entrypoint invoked automatically at contract creation; never exposed as a
/// callable client method (a deployed contract cannot be re-constructed).
const CONSTRUCTOR_FN: &str = "__constructor";

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
/// Deliberately duplicated rather than shared with `bindings ts` / `spec` /
/// `verify`: modules depend only on `soroban-forge-core`, never on each
/// other.
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
/// otherwise the release build of the cargo project in `dir`.
fn resolve_wasm(dir: &Path, wasm_override: Option<&Path>) -> Result<PathBuf> {
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

/// Ask the official CLI for a wasm's interface as JSON.
///
/// Thin system-touching wrapper; not unit-tested.
fn read_interface_json(wasm: &Path) -> Result<String> {
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

// --- Spec parsing ---

/// A public entrypoint: `name(arg: type, ...) -> type`, minus `__constructor`.
struct Entrypoint {
    name: String,
    inputs: Vec<(String, Value)>,
    outputs: Vec<Value>,
}

/// A `#[contracttype] struct`: named fields.
struct StructDef {
    name: String,
    fields: Vec<(String, Value)>,
}

/// One case of a tagged enum (`udt_union_v0`).
enum UnionCase {
    /// A unit variant: no associated data.
    Void(String),
    /// A variant with positional associated types (arity is `types.len()`,
    /// which may be zero, one, or many — see the module docs on how arity
    /// maps to `scval.to_enum`/`from_enum`'s own asymmetric shape).
    Tuple(String, Vec<Value>),
}

/// A `#[contracttype]` tagged enum (Rust `enum` with unit and/or
/// tuple-style variants).
struct UnionDef {
    name: String,
    cases: Vec<UnionCase>,
}

/// A plain (`udt_enum_v0`) or error (`udt_error_enum_v0`) enum: named
/// integer constants. Both kinds share this shape and both render the same
/// way, as an `IntEnum`.
struct EnumDef {
    name: String,
    cases: Vec<(String, i64)>,
}

/// Every declared type and entrypoint pulled out of an interface spec.
#[derive(Default)]
struct Spec {
    entrypoints: Vec<Entrypoint>,
    structs: Vec<StructDef>,
    unions: Vec<UnionDef>,
    enums: Vec<EnumDef>,
}

/// Parse `stellar contract info interface --output json` into every
/// function and type declaration it contains.
fn parse_spec(spec_json: &str) -> Result<Spec> {
    let entries: Value = serde_json::from_str(spec_json)
        .map_err(|e| ForgeError::Other(format!("could not parse contract spec JSON: {e}")))?;
    let entries = entries
        .as_array()
        .ok_or_else(|| ForgeError::Other("contract spec JSON is not a list of entries".into()))?;

    let mut spec = Spec::default();
    for entry in entries {
        if let Some(func) = entry.get("function_v0") {
            let name = func["name"].as_str().unwrap_or_default().to_string();
            if name == CONSTRUCTOR_FN {
                continue;
            }
            let inputs = func["inputs"]
                .as_array()
                .map(|inputs| {
                    inputs
                        .iter()
                        .map(|input| {
                            (
                                input["name"].as_str().unwrap_or("_").to_string(),
                                input["type"].clone(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            let outputs = func["outputs"].as_array().cloned().unwrap_or_default();
            spec.entrypoints.push(Entrypoint {
                name,
                inputs,
                outputs,
            });
        } else if let Some(s) = entry.get("udt_struct_v0") {
            let name = s["name"].as_str().unwrap_or_default().to_string();
            let fields = s["fields"]
                .as_array()
                .map(|fields| {
                    fields
                        .iter()
                        .map(|f| {
                            (
                                f["name"].as_str().unwrap_or_default().to_string(),
                                f["type"].clone(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            spec.structs.push(StructDef { name, fields });
        } else if let Some(u) = entry.get("udt_union_v0") {
            let name = u["name"].as_str().unwrap_or_default().to_string();
            let cases = u["cases"]
                .as_array()
                .map(|cases| {
                    cases
                        .iter()
                        .filter_map(|c| {
                            if let Some(v) = c.get("void_v0") {
                                Some(UnionCase::Void(
                                    v["name"].as_str().unwrap_or_default().to_string(),
                                ))
                            } else if let Some(t) = c.get("tuple_v0") {
                                let types = t["type"].as_array().cloned().unwrap_or_default();
                                Some(UnionCase::Tuple(
                                    t["name"].as_str().unwrap_or_default().to_string(),
                                    types,
                                ))
                            } else {
                                // A struct-like (named-field) union case: not emitted by
                                // soroban-sdk today. Skipped rather than guessed at, so an
                                // unrecognised future case fails loudly in generated code
                                // (a missing variant) instead of silently mis-rendering one.
                                None
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            spec.unions.push(UnionDef { name, cases });
        } else if let Some(e) = entry
            .get("udt_enum_v0")
            .or_else(|| entry.get("udt_error_enum_v0"))
        {
            let name = e["name"].as_str().unwrap_or_default().to_string();
            let cases = e["cases"]
                .as_array()
                .map(|cases| {
                    cases
                        .iter()
                        .map(|c| {
                            (
                                c["name"].as_str().unwrap_or_default().to_string(),
                                c["value"].as_i64().unwrap_or(0),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            spec.enums.push(EnumDef { name, cases });
        }
    }
    Ok(spec)
}

// --- Type rendering ---

/// Names of every declared struct/union/enum, so a `udt` reference can be
/// told apart from one that (for whatever reason) has no definition in this
/// spec, and rendered as `Any` rather than a name that doesn't exist.
struct KnownTypes {
    names: std::collections::HashSet<String>,
}

impl KnownTypes {
    fn from_spec(spec: &Spec) -> Self {
        let mut names = std::collections::HashSet::new();
        names.extend(spec.structs.iter().map(|s| s.name.clone()));
        names.extend(spec.unions.iter().map(|u| u.name.clone()));
        names.extend(spec.enums.iter().map(|e| e.name.clone()));
        Self { names }
    }
}

/// Pull `(kind, inner)` out of a single-key compound type object, e.g.
/// `{"vec": {"element_type": ...}}` -> `("vec", {"element_type": ...})`.
fn compound(ty: &Value) -> Option<(&str, &Value)> {
    ty.as_object()
        .filter(|o| o.len() == 1)
        .and_then(|o| o.iter().next())
        .map(|(k, v)| (k.as_str(), v))
}

/// The Python type hint for a spec type.
fn py_type(ty: &Value, known: &KnownTypes) -> String {
    if let Value::String(name) = ty {
        return match name.as_str() {
            "void" => "None".to_string(),
            "bool" => "bool".to_string(),
            "u32" | "i32" | "u64" | "i64" | "u128" | "i128" | "u256" | "i256" | "timepoint"
            | "duration" => "int".to_string(),
            "bytes" => "bytes".to_string(),
            // `stellar_sdk.scval.from_string` returns `bytes`, not `str` — Soroban's
            // `String` has no UTF-8 guarantee. `Symbol` is ASCII and does decode to `str`.
            "string" => "bytes".to_string(),
            "symbol" => "str".to_string(),
            "address" | "muxed_address" => "Address".to_string(),
            other => other.to_string(), // forward-compatible: an unknown scalar name, verbatim
        };
    }
    let Some((kind, inner)) = compound(ty) else {
        return "Any".to_string();
    };
    match kind {
        "udt" => {
            let name = inner["name"].as_str().unwrap_or("Any");
            if known.names.contains(name) {
                format!("\"{name}\"")
            } else {
                "Any".to_string()
            }
        }
        "vec" => format!("list[{}]", py_type(&inner["element_type"], known)),
        "option" => format!("{} | None", py_type(&inner["value_type"], known)),
        "map" => format!(
            "dict[{}, {}]",
            py_type(&inner["key_type"], known),
            py_type(&inner["value_type"], known)
        ),
        // Errors never surface as a decodable value — see the module docs.
        "result" => py_type(&inner["ok_type"], known),
        "tuple" => {
            let types: Vec<String> = inner["value_types"]
                .as_array()
                .map(|types| types.iter().map(|t| py_type(t, known)).collect())
                .unwrap_or_default();
            if types.is_empty() {
                "None".to_string()
            } else {
                format!("tuple[{}]", types.join(", "))
            }
        }
        "bytes_n" => "bytes".to_string(),
        _ => "Any".to_string(),
    }
}

/// Python expression decoding `expr` (an `SCVal`) to the native value
/// `py_type` describes.
fn decode_expr(ty: &Value, expr: &str) -> String {
    if let Value::String(name) = ty {
        return match name.as_str() {
            "void" => format!("scval.from_void({expr})"),
            "bool" => format!("scval.from_bool({expr})"),
            "u32" => format!("scval.from_uint32({expr})"),
            "i32" => format!("scval.from_int32({expr})"),
            "u64" => format!("scval.from_uint64({expr})"),
            "i64" => format!("scval.from_int64({expr})"),
            "u128" => format!("scval.from_uint128({expr})"),
            "i128" => format!("scval.from_int128({expr})"),
            "u256" => format!("scval.from_uint256({expr})"),
            "i256" => format!("scval.from_int256({expr})"),
            "timepoint" => format!("scval.from_timepoint({expr})"),
            "duration" => format!("scval.from_duration({expr})"),
            "bytes" => format!("scval.from_bytes({expr})"),
            "string" => format!("scval.from_string({expr})"),
            "symbol" => format!("scval.from_symbol({expr})"),
            "address" | "muxed_address" => format!("scval.from_address({expr})"),
            _ => expr.to_string(),
        };
    }
    let Some((kind, inner)) = compound(ty) else {
        return expr.to_string();
    };
    match kind {
        "udt" => format!(
            "{}._decode({expr})",
            inner["name"].as_str().unwrap_or("Any")
        ),
        "vec" => {
            let item = decode_expr(&inner["element_type"], "_e");
            format!("[{item} for _e in scval.from_vec({expr})]")
        }
        "option" => {
            let inner_expr = decode_expr(&inner["value_type"], expr);
            format!("(None if {expr}.type == stellar_xdr.SCValType.SCV_VOID else {inner_expr})")
        }
        "map" => {
            let k = decode_expr(&inner["key_type"], "_k");
            let v = decode_expr(&inner["value_type"], "_v");
            format!("{{{k}: {v} for _k, _v in scval.from_map({expr}).items()}}")
        }
        "result" => decode_expr(&inner["ok_type"], expr),
        "tuple" => {
            let types = inner["value_types"].as_array().cloned().unwrap_or_default();
            if types.is_empty() {
                "None".to_string()
            } else {
                let items: Vec<String> = types
                    .iter()
                    .enumerate()
                    .map(|(i, t)| decode_expr(t, &format!("_t[{i}]")))
                    .collect();
                format!(
                    "(lambda _t: ({}))(scval.from_vec({expr}))",
                    items.join(", ")
                )
            }
        }
        "bytes_n" => format!("scval.from_bytes({expr})"),
        _ => expr.to_string(),
    }
}

/// Python expression encoding `expr` (a native value) to an `SCVal`.
fn encode_expr(ty: &Value, expr: &str) -> String {
    if let Value::String(name) = ty {
        return match name.as_str() {
            "void" => "scval.to_void()".to_string(),
            "bool" => format!("scval.to_bool({expr})"),
            "u32" => format!("scval.to_uint32({expr})"),
            "i32" => format!("scval.to_int32({expr})"),
            "u64" => format!("scval.to_uint64({expr})"),
            "i64" => format!("scval.to_int64({expr})"),
            "u128" => format!("scval.to_uint128({expr})"),
            "i128" => format!("scval.to_int128({expr})"),
            "u256" => format!("scval.to_uint256({expr})"),
            "i256" => format!("scval.to_int256({expr})"),
            "timepoint" => format!("scval.to_timepoint({expr})"),
            "duration" => format!("scval.to_duration({expr})"),
            "bytes" => format!("scval.to_bytes({expr})"),
            "string" => format!("scval.to_string({expr})"),
            "symbol" => format!("scval.to_symbol({expr})"),
            "address" | "muxed_address" => format!("scval.to_address({expr})"),
            _ => expr.to_string(),
        };
    }
    let Some((kind, inner)) = compound(ty) else {
        return expr.to_string();
    };
    match kind {
        "udt" => format!("{expr}._encode()"),
        "vec" => {
            let item = encode_expr(&inner["element_type"], "_e");
            format!("scval.to_vec([{item} for _e in {expr}])")
        }
        "option" => {
            let inner_expr = encode_expr(&inner["value_type"], expr);
            format!("(scval.to_void() if {expr} is None else {inner_expr})")
        }
        "map" => {
            let k = encode_expr(&inner["key_type"], "_k");
            let v = encode_expr(&inner["value_type"], "_v");
            format!("scval.to_map({{{k}: {v} for _k, _v in {expr}.items()}})")
        }
        "result" => encode_expr(&inner["ok_type"], expr),
        "tuple" => {
            let types = inner["value_types"].as_array().cloned().unwrap_or_default();
            let items: Vec<String> = types
                .iter()
                .enumerate()
                .map(|(i, t)| encode_expr(t, &format!("{expr}[{i}]")))
                .collect();
            format!("scval.to_vec([{}])", items.join(", "))
        }
        "bytes_n" => format!("scval.to_bytes({expr})"),
        _ => expr.to_string(),
    }
}

// --- Code rendering ---

/// Every Python 3 reserved keyword. Spec names are Rust identifiers, so the
/// only real risk is an English word that happens to be reserved in Python
/// but not Rust — `from` is the one that actually occurs, in argument names
/// like `from: Address` on a token's `transfer`.
const PY_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

/// A spec-provided name (entrypoint, parameter, or struct field), escaped
/// with a trailing underscore if it collides with a Python keyword, so it
/// is always safe to emit as a Python identifier — `from` becomes `from_`,
/// matching Python's own convention for this exact situation (e.g.
/// `dataclasses.field`'s `from_` isn't a thing, but the convention of
/// suffixing a keyword-colliding name with `_` is standard, see PEP 8).
/// Never applied to the dict/JSON keys used to address on-chain fields by
/// name (`_f["from"]`), only to actual Python identifiers.
fn py_ident(name: &str) -> String {
    if PY_KEYWORDS.contains(&name) {
        format!("{name}_")
    } else {
        name.to_string()
    }
}

fn render_struct(def: &StructDef, known: &KnownTypes) -> String {
    let mut out = format!("@dataclass\nclass {}:\n", def.name);
    if def.fields.is_empty() {
        out.push_str("    pass\n\n");
    } else {
        for (name, ty) in &def.fields {
            out.push_str(&format!("    {}: {}\n", py_ident(name), py_type(ty, known)));
        }
        out.push('\n');
    }
    out.push_str(&format!(
        "    @staticmethod\n    def _decode(sc: SCVal) -> \"{name}\":\n        _f = scval.from_struct(sc)\n",
        name = def.name
    ));
    if def.fields.is_empty() {
        out.push_str(&format!("        return {}()\n\n", def.name));
    } else {
        out.push_str(&format!("        return {}(\n", def.name));
        for (name, ty) in &def.fields {
            out.push_str(&format!(
                "            {}={},\n",
                py_ident(name),
                decode_expr(ty, &format!("_f[\"{name}\"]"))
            ));
        }
        out.push_str("        )\n\n");
    }
    out.push_str("    def _encode(self) -> SCVal:\n        return scval.to_struct({\n");
    for (name, ty) in &def.fields {
        out.push_str(&format!(
            "            \"{name}\": {},\n",
            encode_expr(ty, &format!("self.{}", py_ident(name)))
        ));
    }
    out.push_str("        })\n\n\n");
    out
}

fn render_enum(def: &EnumDef) -> String {
    let mut out = format!("class {}(IntEnum):\n", def.name);
    for (name, value) in &def.cases {
        out.push_str(&format!("    {name} = {value}\n"));
    }
    out.push_str("\n\n");
    out
}

/// A tagged enum: one dataclass per case (a `values: tuple[...]` field for a
/// `Tuple` case, no field at all for a `Void` case) plus a `Union` alias and
/// module-level `_decode_<Name>`/`_encode_<Name>` dispatchers. See the
/// module docs for why every case's data is a tuple regardless of arity,
/// bridging `scval.to_enum`/`from_enum`'s own bare/list/`None` asymmetry.
fn render_union(def: &UnionDef, known: &KnownTypes) -> String {
    let mut out = String::new();
    let mut variant_names = Vec::new();

    for case in &def.cases {
        match case {
            UnionCase::Void(name) => {
                let cls = format!("{}_{name}", def.name);
                variant_names.push((cls.clone(), name.clone()));
                out.push_str(&format!(
                    "@dataclass\nclass {cls}:\n    TAG: ClassVar[str] = \"{name}\"\n\n\n"
                ));
            }
            UnionCase::Tuple(name, types) => {
                let cls = format!("{}_{name}", def.name);
                variant_names.push((cls.clone(), name.clone()));
                let type_list = types
                    .iter()
                    .map(|t| py_type(t, known))
                    .collect::<Vec<_>>()
                    .join(", ");
                out.push_str(&format!(
                    "@dataclass\nclass {cls}:\n    TAG: ClassVar[str] = \"{name}\"\n    values: tuple[{type_list}]\n\n\n"
                ));
            }
        }
    }

    let union_members = variant_names
        .iter()
        .map(|(cls, _)| cls.clone())
        .collect::<Vec<_>>()
        .join(", ");
    out.push_str(&format!("{} = Union[{union_members}]\n\n", def.name));

    out.push_str(&format!(
        "def _decode_{name}(sc: SCVal) -> \"{name}\":\n    _tag, _data = scval.from_enum(sc)\n",
        name = def.name
    ));
    for case in &def.cases {
        match case {
            UnionCase::Void(name) => {
                out.push_str(&format!(
                    "    if _tag == \"{name}\":\n        return {}_{name}()\n",
                    def.name
                ));
            }
            UnionCase::Tuple(name, types) => {
                out.push_str(&format!("    if _tag == \"{name}\":\n"));
                // `scval.from_enum`'s `_data` is statically `SCVal | list[SCVal] | None`
                // — it is only a bare `SCVal` for exactly one value, a `list[SCVal]`
                // for two or more, `None` for a void case. mypy cannot narrow that
                // from the string comparison above, so each arm asserts the shape
                // its own arity guarantees at runtime before indexing/using it.
                let values = match types.len() {
                    0 => String::new(),
                    1 => {
                        out.push_str("        assert isinstance(_data, SCVal)\n");
                        decode_expr(&types[0], "_data")
                    }
                    _ => {
                        out.push_str("        assert isinstance(_data, list)\n");
                        types
                            .iter()
                            .enumerate()
                            .map(|(i, t)| decode_expr(t, &format!("_data[{i}]")))
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                };
                let tuple_literal = if types.len() == 1 {
                    format!("({values},)")
                } else {
                    format!("({values})")
                };
                out.push_str(&format!(
                    "        return {}_{name}({})\n",
                    def.name,
                    if types.is_empty() {
                        String::new()
                    } else {
                        format!("values={tuple_literal}")
                    }
                ));
            }
        }
    }
    out.push_str(&format!(
        "    raise ValueError(f\"unknown {name} variant: {{_tag}}\")\n\n\n",
        name = def.name
    ));

    out.push_str(&format!(
        "def _encode_{name}(value: \"{name}\") -> SCVal:\n",
        name = def.name
    ));
    for case in &def.cases {
        let (cls_suffix, types, is_void) = match case {
            UnionCase::Void(name) => (name.clone(), vec![], true),
            UnionCase::Tuple(name, types) => (name.clone(), types.clone(), false),
        };
        out.push_str(&format!(
            "    if isinstance(value, {}_{cls_suffix}):\n",
            def.name
        ));
        let data_arg = if is_void || types.is_empty() {
            "None".to_string()
        } else if types.len() == 1 {
            encode_expr(&types[0], "value.values[0]")
        } else {
            let items: Vec<String> = types
                .iter()
                .enumerate()
                .map(|(i, t)| encode_expr(t, &format!("value.values[{i}]")))
                .collect();
            format!("[{}]", items.join(", "))
        };
        out.push_str(&format!(
            "        return scval.to_enum(\"{cls_suffix}\", {data_arg})\n"
        ));
    }
    out.push_str(&format!(
        "    raise TypeError(f\"not a {name} variant: {{value!r}}\")\n\n\n",
        name = def.name
    ));
    out
}

/// A read-position type: an `Awaited[T]`-equivalent used for a return type
/// (no encoding needed the other way).
fn render_entrypoint(ep: &Entrypoint, known: &KnownTypes) -> String {
    let params: Vec<String> = ep
        .inputs
        .iter()
        .map(|(name, ty)| format!("{}: {}", py_ident(name), py_type(ty, known)))
        .collect();
    let return_ty = match ep.outputs.as_slice() {
        [] => "None".to_string(),
        [single] => py_type(single, known),
        many => format!(
            "tuple[{}]",
            many.iter()
                .map(|t| py_type(t, known))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let mut out = format!(
        "    def {name}(self, {params}{comma}*, source: str | MuxedAccount = NULL_ACCOUNT, signer: Keypair | None = None) -> AssembledTransaction[{return_ty}]:\n",
        name = ep.name,
        params = params.join(", "),
        comma = if params.is_empty() { "" } else { ", " },
    );
    let parse_fn = match ep.outputs.as_slice() {
        [] => "lambda _sc: None".to_string(),
        [single] => format!("lambda _sc: {}", decode_expr(single, "_sc")),
        many => {
            let items: Vec<String> = many
                .iter()
                .enumerate()
                .map(|(i, t)| decode_expr(t, &format!("_sc_vec[{i}]")))
                .collect();
            format!(
                "lambda _sc: (lambda _sc_vec: ({}))(scval.from_vec(_sc))",
                items.join(", ")
            )
        }
    };
    out.push_str("        return self._client.invoke(\n");
    out.push_str(&format!("            \"{}\",\n", ep.name));
    if ep.inputs.is_empty() {
        out.push_str("            [],\n");
    } else {
        out.push_str("            [\n");
        for (name, ty) in &ep.inputs {
            out.push_str(&format!(
                "                {},\n",
                encode_expr(ty, &py_ident(name))
            ));
        }
        out.push_str("            ],\n");
    }
    out.push_str("            source=source,\n");
    out.push_str("            signer=signer,\n");
    out.push_str(&format!("            parse_result_xdr_fn={parse_fn},\n"));
    out.push_str("        )\n\n");
    out
}

const CLIENT_PREAMBLE: &str = r#"# Generated by `soroban-forge bindings-py`. Do not edit by hand — regenerate
# after the contract's interface changes.
#
# The runtime (transaction assembly, simulation, signing and submission, and
# SCVal encoding/decoding) is never reimplemented here: every method below
# delegates to the official `stellar-sdk` package's
# `stellar_sdk.contract.ContractClient`/`AssembledTransaction` and
# `stellar_sdk.scval`. This file only supplies the per-entrypoint, per-type
# surface `stellar-sdk` needs to know which conversions to run.
#
# `Result<T, E>` types render as plain `T`: a Soroban contract function
# returning `Result<T, E>` traps host-side on `Err` rather than returning a
# decodable error value, so on a successful call only `T` is ever on the
# wire — the error path is already an exception raised by `stellar-sdk`.
from __future__ import annotations

from dataclasses import dataclass
from enum import IntEnum
from typing import Any, ClassVar, Union

from stellar_sdk import Address, Keypair, MuxedAccount
from stellar_sdk import xdr as stellar_xdr
from stellar_sdk import scval
from stellar_sdk.contract import ContractClient, AssembledTransaction
from stellar_sdk.xdr import SCVal

NULL_ACCOUNT = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF"


"#;

/// Render `client.py` from a parsed spec.
fn render_client_py(spec: &Spec) -> String {
    let known = KnownTypes::from_spec(spec);
    let mut out = CLIENT_PREAMBLE.to_string();

    for def in &spec.enums {
        out.push_str(&render_enum(def));
    }
    for def in &spec.structs {
        out.push_str(&render_struct(def, &known));
    }
    for def in &spec.unions {
        out.push_str(&render_union(def, &known));
    }

    out.push_str("class Client:\n");
    out.push_str(
        "    \"\"\"Typed client for this contract. Wraps `stellar_sdk.contract.ContractClient`.\"\"\"\n\n",
    );
    out.push_str("    def __init__(self, contract_id: str, rpc_url: str, network_passphrase: str) -> None:\n");
    out.push_str(
        "        self._client = ContractClient(contract_id, rpc_url, network_passphrase)\n\n",
    );
    for ep in &spec.entrypoints {
        out.push_str(&render_entrypoint(ep, &known));
    }
    out
}

/// Generate `client.py` (plus a `py.typed` marker) for the contract in
/// `contract_dir` into `output`. `wasm_override`, when given, is used
/// instead of auto-detecting the built wasm. Returns the wasm path that was
/// used.
pub fn generate_bindings(
    contract_dir: &Path,
    wasm_override: Option<&Path>,
    output: &Path,
    force: bool,
) -> Result<PathBuf> {
    let wasm_path = resolve_wasm(contract_dir, wasm_override)?;

    if output.exists() && !force {
        return Err(ForgeError::AlreadyExists(output.to_path_buf()));
    }
    std::fs::create_dir_all(output)
        .map_err(ForgeError::io(format!("creating {}", output.display())))?;

    let spec = parse_spec(&read_interface_json(&wasm_path)?)?;
    let client_py = render_client_py(&spec);
    std::fs::write(output.join("client.py"), client_py).map_err(ForgeError::io(format!(
        "writing {}",
        output.join("client.py").display()
    )))?;
    // PEP 561 marker so type checkers treat this directory as typed.
    std::fs::write(output.join("py.typed"), "").map_err(ForgeError::io(format!(
        "writing {}",
        output.join("py.typed").display()
    )))?;

    Ok(wasm_path)
}

/// The `bindings-py` subcommand.
///
/// A separate top-level command rather than a `bindings py` sub-subcommand:
/// `soroban-forge-core`'s plugin dispatch maps one top-level subcommand name
/// to exactly one plugin, and `soroban-forge-bindings-ts` already owns
/// `bindings` (with `ts` as its only sub-subcommand). Adding `py` there
/// would mean this crate depending on `bindings-ts`, which the "modules
/// never depend on each other" rule (see CONTRIBUTING.md) rules out.
pub struct BindingsPyPlugin;

impl ForgePlugin for BindingsPyPlugin {
    fn name(&self) -> &'static str {
        "bindings-py"
    }

    fn command(&self) -> Command {
        Command::new("bindings-py")
            .about("Generate a typed Python client package from the built contract wasm")
            .long_about(
                "Generate client.py: a typed Python client, one method per entrypoint, from \
                 the built contract wasm.\n\n\
                 Unlike `bindings ts`, this is not a `stellar-cli` wrapper — `stellar contract \
                 bindings python` is unimplemented, delegating instead to a third-party tool \
                 that only works against a deployed contract. This command reads the interface \
                 the same official way `spec`/`verify` do and renders Python source directly, \
                 with the generated client delegating all on-chain interaction to the official \
                 `stellar-sdk` package.",
            )
            .arg(
                Arg::new("path")
                    .long("path")
                    .help("Contract project directory [default: current directory]"),
            )
            .arg(Arg::new("wasm").long("wasm").help(
                "Path to the built .wasm file [default: target/wasm32v1-none/release/<crate>.wasm]",
            ))
            .arg(
                Arg::new("output")
                    .long("output")
                    .short('o')
                    .help("Output directory for the generated package [default: bindings/python]"),
            )
            .arg(
                Arg::new("force")
                    .long("force")
                    .action(ArgAction::SetTrue)
                    .help("Overwrite the output directory if it exists"),
            )
    }

    fn run(&self, matches: &ArgMatches, ctx: &ForgeContext) -> Result<()> {
        let dir = matches
            .get_one::<String>("path")
            .map(|p| ctx.cwd.join(p))
            .unwrap_or_else(|| ctx.cwd.clone());
        let wasm_override = matches.get_one::<String>("wasm").map(|p| ctx.cwd.join(p));
        let output = matches
            .get_one::<String>("output")
            .map(|p| ctx.cwd.join(p))
            .unwrap_or_else(|| dir.join(DEFAULT_OUTPUT_SUBDIR));
        let force = matches.get_flag("force");

        let wasm_path = generate_bindings(&dir, wasm_override.as_deref(), &output, force)?;

        if ctx.json {
            let report = serde_json::json!({
                "wasm_path": wasm_path.display().to_string(),
                "output_dir": output.display().to_string(),
            });
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
        } else {
            println!("generated Python bindings from {}", wasm_path.display());
            println!("  -> {}", output.join("client.py").display());
            println!();
            println!("next steps:");
            println!("  pip install stellar-sdk");
            println!("  python -c 'from client import Client'");
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
        assert!(read_crate_name(tmp.path()).is_err());
    }

    #[test]
    fn missing_wasm_points_at_stellar_contract_build() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        let output = tmp.path().join("bindings/python");
        let err = generate_bindings(tmp.path(), None, &output, false).unwrap_err();
        assert!(err.to_string().contains("stellar contract build"), "{err}");
    }

    #[test]
    fn refuses_to_overwrite_output_without_force() {
        let tmp = tempfile::tempdir().unwrap();
        let wasm_dir = tmp.path().join("target/wasm32v1-none/release");
        std::fs::create_dir_all(&wasm_dir).unwrap();
        let wasm = wasm_dir.join("custom.wasm");
        std::fs::write(&wasm, b"\0asm").unwrap();
        let output = tmp.path().join("bindings/python");
        std::fs::create_dir_all(&output).unwrap();

        let err = generate_bindings(tmp.path(), Some(&wasm), &output, false).unwrap_err();
        assert!(matches!(err, ForgeError::AlreadyExists(_)));
    }

    fn spec_json(text: &str) -> Spec {
        parse_spec(text).unwrap()
    }

    #[test]
    fn parses_functions_and_excludes_the_constructor() {
        let spec = spec_json(
            r#"[
                {"function_v0": {"doc": "", "name": "__constructor", "inputs": [], "outputs": []}},
                {"function_v0": {"doc": "", "name": "mint", "inputs": [{"doc":"","name":"to","type":"address"}], "outputs": []}}
            ]"#,
        );
        assert_eq!(spec.entrypoints.len(), 1);
        assert_eq!(spec.entrypoints[0].name, "mint");
    }

    #[test]
    fn parses_a_struct() {
        let spec = spec_json(
            r#"[{"udt_struct_v0": {"doc":"","lib":"","name":"Proposal","fields":[
                {"doc":"","name":"id","type":"u64"},
                {"doc":"","name":"executed","type":"bool"}
            ]}}]"#,
        );
        assert_eq!(spec.structs.len(), 1);
        assert_eq!(spec.structs[0].name, "Proposal");
        assert_eq!(spec.structs[0].fields.len(), 2);
    }

    #[test]
    fn parses_a_union_with_void_and_tuple_cases() {
        let spec = spec_json(
            r#"[{"udt_union_v0": {"doc":"","lib":"","name":"DataKey","cases":[
                {"tuple_v0": {"doc":"","name":"Proposal","type":["u64"]}},
                {"tuple_v0": {"doc":"","name":"Voted","type":["u64","address"]}},
                {"void_v0": {"doc":"","name":"NextId"}}
            ]}}]"#,
        );
        assert_eq!(spec.unions.len(), 1);
        assert_eq!(spec.unions[0].cases.len(), 3);
    }

    #[test]
    fn parses_an_error_enum() {
        let spec = spec_json(
            r#"[{"udt_error_enum_v0": {"doc":"","lib":"","name":"Error","cases":[
                {"doc":"","name":"NotFound","value":1},
                {"doc":"","name":"Unauthorized","value":2}
            ]}}]"#,
        );
        assert_eq!(spec.enums.len(), 1);
        assert_eq!(
            spec.enums[0].cases,
            vec![("NotFound".to_string(), 1), ("Unauthorized".to_string(), 2)]
        );
    }

    #[test]
    fn malformed_spec_json_is_an_error_not_a_panic() {
        assert!(parse_spec("not json").is_err());
        assert!(parse_spec("{}").is_err());
    }

    fn known(names: &[&str]) -> KnownTypes {
        KnownTypes {
            names: names.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn primitive_type_hints() {
        let k = known(&[]);
        assert_eq!(py_type(&Value::String("bool".into()), &k), "bool");
        assert_eq!(py_type(&Value::String("i128".into()), &k), "int");
        assert_eq!(py_type(&Value::String("string".into()), &k), "bytes");
        assert_eq!(py_type(&Value::String("symbol".into()), &k), "str");
        assert_eq!(py_type(&Value::String("address".into()), &k), "Address");
        assert_eq!(
            py_type(&Value::String("muxed_address".into()), &k),
            "Address"
        );
    }

    #[test]
    fn compound_type_hints() {
        let k = known(&["Proposal"]);
        assert_eq!(
            py_type(&serde_json::json!({"vec": {"element_type": "u32"}}), &k),
            "list[int]"
        );
        assert_eq!(
            py_type(&serde_json::json!({"option": {"value_type": "bool"}}), &k),
            "bool | None"
        );
        assert_eq!(
            py_type(
                &serde_json::json!({"map": {"key_type": "symbol", "value_type": "i128"}}),
                &k
            ),
            "dict[str, int]"
        );
        assert_eq!(
            py_type(&serde_json::json!({"udt": {"name": "Proposal"}}), &k),
            "\"Proposal\""
        );
        assert_eq!(
            py_type(&serde_json::json!({"udt": {"name": "Missing"}}), &k),
            "Any"
        );
        assert_eq!(
            py_type(
                &serde_json::json!({"result": {"ok_type": "u32", "error_type": {"udt": {"name": "Error"}}}}),
                &k
            ),
            "int"
        );
    }

    #[test]
    fn decode_and_encode_primitives() {
        assert_eq!(
            decode_expr(&Value::String("i128".into()), "sc"),
            "scval.from_int128(sc)"
        );
        assert_eq!(
            encode_expr(&Value::String("i128".into()), "x"),
            "scval.to_int128(x)"
        );
        assert_eq!(
            decode_expr(&Value::String("address".into()), "sc"),
            "scval.from_address(sc)"
        );
    }

    #[test]
    fn decode_option_checks_void_type() {
        let ty = serde_json::json!({"option": {"value_type": "u32"}});
        let expr = decode_expr(&ty, "sc");
        assert!(expr.contains("SCV_VOID"), "{expr}");
        assert!(expr.contains("scval.from_uint32(sc)"), "{expr}");
    }

    #[test]
    fn encode_option_checks_none() {
        let ty = serde_json::json!({"option": {"value_type": "u32"}});
        let expr = encode_expr(&ty, "x");
        assert!(expr.contains("x is None"), "{expr}");
        assert!(expr.contains("scval.to_uint32(x)"), "{expr}");
    }

    #[test]
    fn render_struct_produces_dataclass_and_codec_methods() {
        let def = StructDef {
            name: "Proposal".into(),
            fields: vec![
                ("id".into(), Value::String("u64".into())),
                ("executed".into(), Value::String("bool".into())),
            ],
        };
        let out = render_struct(&def, &known(&["Proposal"]));
        assert!(out.contains("@dataclass"), "{out}");
        assert!(out.contains("class Proposal:"), "{out}");
        assert!(out.contains("id: int"), "{out}");
        assert!(out.contains("executed: bool"), "{out}");
        assert!(out.contains("def _decode(sc: SCVal)"), "{out}");
        assert!(out.contains("def _encode(self)"), "{out}");
    }

    #[test]
    fn render_union_covers_void_and_tuple_cases() {
        let def = UnionDef {
            name: "DataKey".into(),
            cases: vec![
                UnionCase::Tuple("Proposal".into(), vec![Value::String("u64".into())]),
                UnionCase::Void("NextId".into()),
            ],
        };
        let out = render_union(&def, &known(&[]));
        assert!(out.contains("class DataKey_Proposal:"), "{out}");
        assert!(out.contains("values: tuple[int]"), "{out}");
        assert!(out.contains("class DataKey_NextId:"), "{out}");
        assert!(
            out.contains("DataKey = Union[DataKey_Proposal, DataKey_NextId]"),
            "{out}"
        );
        assert!(out.contains("assert isinstance(_data, SCVal)"), "{out}");
        assert!(out.contains("def _decode_DataKey(sc: SCVal)"), "{out}");
        assert!(out.contains("def _encode_DataKey(value:"), "{out}");
    }

    #[test]
    fn render_union_multi_value_case_narrows_data_to_a_list() {
        let def = UnionDef {
            name: "DataKey".into(),
            cases: vec![UnionCase::Tuple(
                "Voted".into(),
                vec![Value::String("u64".into()), Value::String("address".into())],
            )],
        };
        let out = render_union(&def, &known(&[]));
        assert!(out.contains("assert isinstance(_data, list)"), "{out}");
        assert!(
            out.contains("_data[0]") && out.contains("_data[1]"),
            "{out}"
        );
    }

    #[test]
    fn render_entrypoint_builds_invoke_call() {
        let ep = Entrypoint {
            name: "mint".into(),
            inputs: vec![
                ("to".into(), Value::String("address".into())),
                ("amount".into(), Value::String("i128".into())),
            ],
            outputs: vec![],
        };
        let out = render_entrypoint(&ep, &known(&[]));
        assert!(
            out.contains("def mint(self, to: Address, amount: int"),
            "{out}"
        );
        assert!(out.contains("\"mint\""), "{out}");
        assert!(out.contains("scval.to_address(to)"), "{out}");
        assert!(out.contains("scval.to_int128(amount)"), "{out}");
        assert!(out.contains("AssembledTransaction[None]"), "{out}");
    }

    #[test]
    fn py_ident_escapes_keywords_only() {
        assert_eq!(py_ident("from"), "from_");
        assert_eq!(py_ident("class"), "class_");
        assert_eq!(py_ident("import"), "import_");
        assert_eq!(py_ident("to"), "to");
        assert_eq!(py_ident("amount"), "amount");
    }

    #[test]
    fn render_entrypoint_escapes_a_python_keyword_argument() {
        // A token's `transfer(from: Address, to: Address, amount: i128)` —
        // `from` is a Python keyword and would otherwise be a syntax error.
        let ep = Entrypoint {
            name: "transfer".into(),
            inputs: vec![
                ("from".into(), Value::String("address".into())),
                ("to".into(), Value::String("address".into())),
            ],
            outputs: vec![],
        };
        let out = render_entrypoint(&ep, &known(&[]));
        assert!(
            out.contains("def transfer(self, from_: Address, to: Address"),
            "{out}"
        );
        assert!(out.contains("scval.to_address(from_)"), "{out}");
        assert!(!out.contains("from: Address"), "{out}");
        // The on-chain method name passed to invoke() is unescaped.
        assert!(out.contains("\"transfer\""), "{out}");
    }

    #[test]
    fn render_struct_escapes_a_python_keyword_field() {
        let def = StructDef {
            name: "AllowanceKey".into(),
            fields: vec![
                ("from".into(), Value::String("address".into())),
                ("spender".into(), Value::String("address".into())),
            ],
        };
        let out = render_struct(&def, &known(&["AllowanceKey"]));
        assert!(out.contains("from_: Address"), "{out}");
        assert!(!out.contains("\n    from: Address"), "{out}");
        assert!(
            out.contains("from_=scval.from_address(_f[\"from\"])"),
            "{out}"
        );
        assert!(
            out.contains("\"from\": scval.to_address(self.from_)"),
            "{out}"
        );
    }

    #[test]
    fn render_entrypoint_with_a_return_type() {
        let ep = Entrypoint {
            name: "balance".into(),
            inputs: vec![("id".into(), Value::String("address".into()))],
            outputs: vec![Value::String("i128".into())],
        };
        let out = render_entrypoint(&ep, &known(&[]));
        assert!(out.contains("AssembledTransaction[int]"), "{out}");
        assert!(out.contains("scval.from_int128(_sc)"), "{out}");
    }

    #[test]
    fn plugin_name_matches_its_command() {
        assert_eq!(
            BindingsPyPlugin.name(),
            BindingsPyPlugin.command().get_name()
        );
    }

    #[test]
    fn help_documents_the_flags() {
        let help = BindingsPyPlugin.command().render_long_help().to_string();
        assert!(help.contains("--path"), "{help}");
        assert!(help.contains("--wasm"), "{help}");
        assert!(help.contains("--output"), "{help}");
        assert!(help.contains("--force"), "{help}");
    }
}
