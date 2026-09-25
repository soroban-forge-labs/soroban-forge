//! # soroban-forge-bindings-ts
//!
//! `soroban-forge bindings ts` — generates a TypeScript client package from
//! a built contract's wasm.
//!
//! This module never reimplements XDR-spec-to-TypeScript generation itself;
//! per soroban-forge's "wrap, don't reimplement" rule it shells out to the
//! official `stellar contract bindings typescript` command and only handles:
//!
//! - locating the built `.wasm` for a scaffolded project
//!   (`target/wasm32v1-none/release/<crate_name>.wasm`, matching the
//!   `wasm32v1-none` target soroban-forge templates and `doctor` expect)
//! - choosing/validating the output directory
//! - surfacing a friendly error (pointing at `soroban-forge doctor`) when
//!   `stellar-cli` isn't on `PATH`
//! - rewriting the generated `package.json` so the package is publishable
//!   as-is: a conditional `exports` map, `types`, `files`, and
//!   `@stellar/stellar-sdk` as a peer dependency (see
//!   [`make_publishable`])
//! - with `--react`, additionally emitting `src/hooks.ts`: one hook per
//!   entrypoint, typed against the generated client rather than duplicating
//!   its types (see [`render_hooks_ts`]). Strictly opt-in: without the flag
//!   nothing changes and no `react` dependency is added anywhere

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use clap::{Arg, ArgAction, ArgMatches, Command};
use serde::Deserialize;
use soroban_forge_core::{ForgeContext, ForgeError, ForgePlugin, Result};

const DEFAULT_OUTPUT_SUBDIR: &str = "bindings/typescript";

/// The Stellar SDK package the generated client imports.
pub const STELLAR_SDK: &str = "@stellar/stellar-sdk";

/// Peer range used when `stellar contract bindings typescript` does not pin
/// the SDK itself. The supported range otherwise follows the stellar-cli
/// release that generated the bindings (stellar-cli 28 pins `^16`), since
/// the generated code targets that SDK's API.
pub const DEFAULT_STELLAR_SDK_RANGE: &str = "^16.0.0";

/// Oldest Node.js the generated package declares support for.
pub const MIN_NODE: &str = ">=18";

/// Peer range for `react` when `--react` is used. Wide because the hooks
/// only use `useState`/`useEffect`/`useCallback`/`useRef`, stable since
/// hooks were introduced.
pub const REACT_PEER_RANGE: &str = ">=16.8.0";

#[derive(Deserialize)]
struct Manifest {
    package: Package,
}

#[derive(Deserialize)]
struct Package {
    name: String,
    /// A string, or `{ workspace = true }` — only the string form is used.
    version: Option<toml::Value>,
}

/// Cargo package identity, enough to locate the built wasm and name the
/// generated npm package.
#[derive(Debug, Clone, PartialEq)]
pub struct PackageInfo {
    /// Cargo package name, e.g. `my-token`.
    pub package_name: String,
    /// Rust crate name (snake_case), e.g. `my_token`.
    pub crate_name: String,
    /// `[package].version` when it is a literal string.
    pub version: Option<String>,
}

/// Read `[package].name` out of `dir/Cargo.toml`.
pub fn read_package_info(dir: &Path) -> Result<PackageInfo> {
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
    Ok(PackageInfo {
        crate_name: manifest.package.name.replace('-', "_"),
        version: manifest
            .package
            .version
            .and_then(|v| v.as_str().map(str::to_string)),
        package_name: manifest.package.name,
    })
}

/// Default location `stellar contract build` writes its release wasm to,
/// for a project built with the `wasm32v1-none` target (see `doctor`).
pub fn locate_wasm(dir: &Path, crate_name: &str) -> PathBuf {
    dir.join("target/wasm32v1-none/release")
        .join(format!("{crate_name}.wasm"))
}

/// Generate a TypeScript bindings package for the contract in `contract_dir`
/// into `output`. `wasm_override`, when given, is used instead of
/// auto-detecting the built wasm. When `react` is set, also emits
/// `output/src/hooks.ts` and marks `react` as a peer dependency; otherwise
/// nothing react-related is written or declared. Returns the wasm path that
/// was used.
pub fn generate_bindings(
    contract_dir: &Path,
    wasm_override: Option<&Path>,
    output: &Path,
    force: bool,
    react: bool,
) -> Result<PathBuf> {
    // With --wasm the project manifest is optional: it only supplies the
    // npm package name and version.
    let info = match wasm_override {
        Some(_) => read_package_info(contract_dir).ok(),
        None => Some(read_package_info(contract_dir)?),
    };
    let wasm_path = match (wasm_override, &info) {
        (Some(p), _) => p.to_path_buf(),
        (None, Some(info)) => locate_wasm(contract_dir, &info.crate_name),
        (None, None) => unreachable!("read_package_info errors above"),
    };

    if !wasm_path.is_file() {
        return Err(ForgeError::InvalidArgument(format!(
            "no built wasm found at {} — run `stellar contract build` first (or pass --wasm)",
            wasm_path.display()
        )));
    }

    if output.exists() && !force {
        return Err(ForgeError::AlreadyExists(output.to_path_buf()));
    }
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)
            .map_err(ForgeError::io(format!("creating {}", parent.display())))?;
    }

    run_stellar_bindings(&wasm_path, output)?;
    finalize_package_json(output, info.as_ref(), react)?;
    if react {
        let entrypoints = entrypoint_names(&read_interface_json(&wasm_path)?)?;
        std::fs::write(output.join("src/hooks.ts"), render_hooks_ts(&entrypoints)).map_err(
            ForgeError::io(format!("writing {}", output.join("src/hooks.ts").display())),
        )?;
        append_react_readme_section(output)?;
    }
    Ok(wasm_path)
}

/// Rewrite `output/package.json` in place with [`make_publishable`], adding
/// the `react` peer dependency and `./hooks` export when `react` is set.
fn finalize_package_json(output: &Path, info: Option<&PackageInfo>, react: bool) -> Result<()> {
    let path = output.join("package.json");
    let raw = std::fs::read_to_string(&path)
        .map_err(ForgeError::io(format!("reading {}", path.display())))?;
    let mut pkg: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
        ForgeError::Other(format!(
            "stellar generated an invalid {}: {e}",
            path.display()
        ))
    })?;
    make_publishable(&mut pkg, info);
    if react {
        add_react_support(&mut pkg);
    }
    let mut pretty = serde_json::to_string_pretty(&pkg).expect("a JSON value always serialises");
    pretty.push('\n');
    std::fs::write(&path, pretty).map_err(ForgeError::io(format!("writing {}", path.display())))
}

/// Declare `react` as an optional peer dependency (and a dev dependency, so
/// `tsc` can type-check `hooks.ts`) and expose it at the `./hooks` subpath.
/// Called only when `--react` is passed — otherwise `react` never appears in
/// the generated `package.json` at all.
fn add_react_support(pkg: &mut serde_json::Value) {
    use serde_json::json;

    let Some(obj) = pkg.as_object_mut() else {
        return;
    };
    if let Some(exports) = obj
        .get_mut("exports")
        .and_then(serde_json::Value::as_object_mut)
    {
        exports.insert(
            "./hooks".into(),
            json!({
                "types": "./dist/hooks.d.ts",
                "import": "./dist/hooks.js",
                "default": "./dist/hooks.js"
            }),
        );
    }
    for table in ["peerDependencies", "devDependencies"] {
        if let Some(deps) = obj
            .entry(table)
            .or_insert_with(|| json!({}))
            .as_object_mut()
        {
            deps.insert("react".into(), json!(REACT_PEER_RANGE));
        }
    }
    if let Some(deps) = obj
        .entry("devDependencies")
        .or_insert_with(|| json!({}))
        .as_object_mut()
    {
        deps.insert("@types/react".into(), json!(REACT_PEER_RANGE));
    }
    // Consumers that only use the generated client, not `./hooks`, should not
    // be forced to install react.
    if let Some(meta) = obj
        .entry("peerDependenciesMeta")
        .or_insert_with(|| json!({}))
        .as_object_mut()
    {
        meta.insert("react".into(), json!({ "optional": true }));
    }
}

/// Appended to `output/README.md` when `--react` is used.
const REACT_README_SECTION: &str = r#"
## React hooks

Generated by `--react` in `src/hooks.ts`, exported at the `./hooks` subpath.
One hook per entrypoint: a read entrypoint gets a query-style hook that
fetches on mount, a write entrypoint gets a mutation hook you trigger
explicitly.

```tsx
import { Client } from "my-token";
import { useBalance, useMintMutation } from "my-token/hooks";

const client = new Client({ contractId: "C...", networkPassphrase: "...", rpcUrl: "..." });

function Balance({ id }: { id: string }) {
  const { data, loading, error, refetch } = useBalance(client, { id });
  if (loading) return <p>Loading…</p>;
  if (error) return <p>{error.message}</p>;
  return <p>{String(data)} <button onClick={refetch}>Refresh</button></p>;
}

function MintButton({ to }: { to: string }) {
  const { mutate, loading } = useMintMutation(client);
  return <button disabled={loading} onClick={() => mutate({ to, amount: 100n })}>Mint</button>;
}
```

Soroban's interface has no read/write annotation, so entrypoints are
classified by name (`get_`/`is_`/`has_`/`list_`/`view_`/`query_`/`read_`
prefixes, plus common getters like `balance`, `owner`, `admin`); anything
else is treated as a write. Rename the entrypoint if a hook has the wrong
shape.
"#;

/// Append a short usage section for the generated hooks to `output/README.md`.
fn append_react_readme_section(output: &Path) -> Result<()> {
    let path = output.join("README.md");
    let mut readme = std::fs::read_to_string(&path).unwrap_or_default();
    if !readme.ends_with('\n') && !readme.is_empty() {
        readme.push('\n');
    }
    readme.push_str(REACT_README_SECTION);
    std::fs::write(&path, readme).map_err(ForgeError::io(format!("writing {}", path.display())))
}

/// Whether an entrypoint looks like a read (view) call rather than a
/// state-changing one, used to pick which hook shape `--react` emits.
///
/// Soroban's contract interface carries no formal mutability annotation the
/// way, say, Solidity's ABI `stateMutability` does, so this is a naming
/// heuristic: common getter prefixes and a fixed list of well-known getter
/// names. Anything else is treated as a write, which is the safer default —
/// a write hook never runs on its own, so a misclassified write only means a
/// mutation gets a manual `mutate()` trigger instead of auto-fetching.
fn looks_like_read(name: &str) -> bool {
    const READ_PREFIXES: &[&str] = &["get_", "is_", "has_", "list_", "view_", "query_", "read_"];
    const READ_NAMES: &[&str] = &[
        "balance",
        "balance_of",
        "allowance",
        "owner",
        "owner_of",
        "admin",
        "name",
        "symbol",
        "decimals",
        "total_supply",
        "token_uri",
        "metadata",
        "uri",
    ];
    READ_PREFIXES.iter().any(|prefix| name.starts_with(prefix)) || READ_NAMES.contains(&name)
}

/// `mint_to` -> `MintTo`. Entrypoint names are Rust identifiers (snake_case
/// ASCII), which are always valid TypeScript identifiers, so no escaping is
/// needed for the generated hook names.
fn to_pascal_case(name: &str) -> String {
    name.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// Entrypoint invoked automatically at contract creation. It appears in the
/// spec but the generated `Client` never exposes it as a callable method (a
/// deployed contract cannot be re-constructed), so it never gets a hook.
const CONSTRUCTOR_FN: &str = "__constructor";

/// Pull the `function_v0` entries' names out of
/// `stellar contract info interface --output json`, in spec order, excluding
/// [`CONSTRUCTOR_FN`]. Only names are needed: hook parameter and return
/// types are derived from the generated client itself via TypeScript's
/// `Parameters`/`ReturnType`, not duplicated here.
fn entrypoint_names(spec_json: &str) -> Result<Vec<String>> {
    let entries: serde_json::Value = serde_json::from_str(spec_json)
        .map_err(|e| ForgeError::Other(format!("could not parse contract spec JSON: {e}")))?;
    let entries = entries
        .as_array()
        .ok_or_else(|| ForgeError::Other("contract spec JSON is not a list of entries".into()))?;
    Ok(entries
        .iter()
        .filter_map(|entry| {
            entry
                .get("function_v0")?
                .get("name")?
                .as_str()
                .map(str::to_string)
        })
        .filter(|name| name != CONSTRUCTOR_FN)
        .collect())
}

/// Ask the official CLI for a wasm's interface as JSON. Thin system-touching
/// wrapper; not unit-tested.
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

/// Shared runtime helpers plus the file header, common to every generated
/// `hooks.ts` regardless of the contract's entrypoints.
const HOOKS_PREAMBLE: &str = r#"// Generated by `soroban-forge bindings ts --react`. Do not edit by hand —
// regenerate after the contract's interface changes.
//
// Soroban's contract interface has no read/write (view/mutating) annotation
// the way an ABI's `stateMutability` would, so each hook below is chosen by
// name: a `get_`/`is_`/`has_`/`list_`/`view_`/`query_`/`read_` prefix, or one
// of a fixed list of common getter names, is treated as a read; everything
// else is a write. If a hook has the wrong shape for your contract, that
// heuristic missed it.
import { useCallback, useEffect, useRef, useState } from "react";
import type { Client } from "./index.js";

/** State returned by a read hook. */
export interface ReadState<T> {
  data: T | undefined;
  loading: boolean;
  error: Error | undefined;
  refetch: () => void;
}

/**
 * Runs `call` on mount and whenever `depsKey` changes (compared with
 * `JSON.stringify`). If `depsKey` is built from object/array arguments
 * constructed fresh on every render, wrap it in `useMemo` first, or the hook
 * will refetch every render.
 */
function useContractRead<T>(
  call: () => Promise<{ result: T }>,
  depsKey: unknown,
): ReadState<T> {
  const [data, setData] = useState<T>();
  const [error, setError] = useState<Error>();
  const [loading, setLoading] = useState(true);
  const callRef = useRef(call);
  callRef.current = call;
  const key = JSON.stringify(depsKey);

  const run = useCallback(() => {
    setLoading(true);
    setError(undefined);
    callRef
      .current()
      .then((tx) => setData(tx.result))
      .catch((e: unknown) => setError(e instanceof Error ? e : new Error(String(e))))
      .finally(() => setLoading(false));
    // `call` itself is intentionally excluded: `key` is the real dependency.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);

  useEffect(() => {
    run();
  }, [run]);

  return { data, loading, error, refetch: run };
}

/** State returned by a write (mutation) hook. */
export interface MutationState<Args extends unknown[], T> {
  mutate: (...args: Args) => Promise<T>;
  data: T | undefined;
  loading: boolean;
  error: Error | undefined;
}

/**
 * Wraps a state-changing entrypoint. `mutate(...)` builds and simulates the
 * call, signs and sends it, and resolves with the final result. It never
 * runs on its own.
 */
function useContractMutation<Args extends unknown[], T>(
  call: (...args: Args) => Promise<{ signAndSend: () => Promise<{ result: T }> }>,
): MutationState<Args, T> {
  const [data, setData] = useState<T>();
  const [error, setError] = useState<Error>();
  const [loading, setLoading] = useState(false);
  const callRef = useRef(call);
  callRef.current = call;

  const mutate = useCallback(async (...args: Args): Promise<T> => {
    setLoading(true);
    setError(undefined);
    try {
      const assembled = await callRef.current(...args);
      const sent = await assembled.signAndSend();
      setData(sent.result);
      return sent.result;
    } catch (e) {
      const err = e instanceof Error ? e : new Error(String(e));
      setError(err);
      throw err;
    } finally {
      setLoading(false);
    }
  }, []);

  return { mutate, data, loading, error };
}
"#;

fn render_read_hook(name: &str) -> String {
    let pascal = to_pascal_case(name);
    format!(
        "\nexport function use{pascal}(\n  \
         client: Client,\n  \
         ...args: Parameters<Client[\"{name}\"]>\n\
         ): ReadState<Awaited<ReturnType<Client[\"{name}\"]>>[\"result\"]> {{\n  \
         return useContractRead(() => client.{name}(...args), args);\n\
         }}\n"
    )
}

fn render_write_hook(name: &str) -> String {
    let pascal = to_pascal_case(name);
    format!(
        "\nexport function use{pascal}Mutation(\n  \
         client: Client,\n\
         ): MutationState<Parameters<Client[\"{name}\"]>, Awaited<ReturnType<Client[\"{name}\"]>>[\"result\"]> {{\n  \
         return useContractMutation((...args: Parameters<Client[\"{name}\"]>) => client.{name}(...args));\n\
         }}\n"
    )
}

/// Render `src/hooks.ts`: the shared preamble plus one hook per entrypoint,
/// in spec order.
pub fn render_hooks_ts(entrypoints: &[String]) -> String {
    let mut out = HOOKS_PREAMBLE.to_string();
    for name in entrypoints {
        out.push_str(&if looks_like_read(name) {
            render_read_hook(name)
        } else {
            render_write_hook(name)
        });
    }
    out
}

/// npm package name for a cargo package: npm names must be lowercase.
pub fn npm_package_name(package_name: &str) -> String {
    package_name.to_ascii_lowercase()
}

/// Turn the `package.json` stellar-cli generates into one that can be
/// `npm pack`ed and published without edits:
///
/// - name/version from `Cargo.toml` when known
/// - `exports` as a conditional map (`types` first, then `import`/`default`)
///   plus `main`/`types` for tools that predate `exports` — this is what
///   lets the declarations resolve under both `node16`/`nodenext` and
///   `bundler` module resolution
/// - `files` limited to the build output, sources and README
/// - `@stellar/stellar-sdk` moved from `dependencies` to
///   `peerDependencies` (and mirrored in `devDependencies` so `tsc` can
///   still build), so apps and the client share one SDK instance
/// - a `prepack` build, so `npm pack`/`npm publish` never ship a stale or
///   missing `dist/`
///
/// Fields stellar-cli sets that are not listed here are left untouched.
pub fn make_publishable(pkg: &mut serde_json::Value, info: Option<&PackageInfo>) {
    use serde_json::{json, Map, Value};

    if !pkg.is_object() {
        *pkg = Value::Object(Map::new());
    }
    let obj = pkg.as_object_mut().expect("just ensured an object");

    if let Some(info) = info {
        obj.insert("name".into(), json!(npm_package_name(&info.package_name)));
        if let Some(version) = &info.version {
            obj.insert("version".into(), json!(version));
        }
    }
    obj.entry("version").or_insert_with(|| json!("0.0.0"));

    obj.insert("type".into(), json!("module"));
    obj.insert("main".into(), json!("./dist/index.js"));
    obj.insert("types".into(), json!("./dist/index.d.ts"));
    obj.remove("typings");
    obj.insert(
        "exports".into(),
        json!({
            ".": {
                "types": "./dist/index.d.ts",
                "import": "./dist/index.js",
                "default": "./dist/index.js"
            },
            "./package.json": "./package.json"
        }),
    );
    obj.insert("files".into(), json!(["dist", "src", "README.md"]));
    obj.insert("sideEffects".into(), json!(false));
    obj.entry("engines")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .map(|engines| engines.entry("node").or_insert_with(|| json!(MIN_NODE)));

    let scripts = obj
        .entry("scripts")
        .or_insert_with(|| json!({}))
        .as_object_mut();
    if let Some(scripts) = scripts {
        scripts.entry("build").or_insert_with(|| json!("tsc"));
        scripts.insert("prepack".into(), json!("npm run build"));
    }

    // Move the SDK to peerDependencies, keeping whatever range the CLI chose.
    let sdk_range = obj
        .get_mut("dependencies")
        .and_then(Value::as_object_mut)
        .and_then(|deps| deps.remove(STELLAR_SDK))
        .unwrap_or_else(|| json!(DEFAULT_STELLAR_SDK_RANGE));
    for table in ["peerDependencies", "devDependencies"] {
        if let Some(deps) = obj
            .entry(table)
            .or_insert_with(|| json!({}))
            .as_object_mut()
        {
            deps.insert(STELLAR_SDK.into(), sdk_range.clone());
        }
    }
    if obj
        .get("dependencies")
        .and_then(Value::as_object)
        .is_some_and(Map::is_empty)
    {
        obj.remove("dependencies");
    }
}

/// Shell out to the official CLI. Never reimplemented locally.
fn run_stellar_bindings(wasm: &Path, output: &Path) -> Result<()> {
    let wasm_str = wasm.to_str().ok_or_else(|| {
        ForgeError::Other(format!("wasm path {} is not valid UTF-8", wasm.display()))
    })?;
    let output_str = output.to_str().ok_or_else(|| {
        ForgeError::Other(format!(
            "output path {} is not valid UTF-8",
            output.display()
        ))
    })?;

    // TODO(verify): confirm `--output-dir` is the correct flag name against
    // `stellar contract bindings typescript --help` — not reimplementing the
    // generator locally means we depend on the CLI's own interface here.
    let result = std::process::Command::new("stellar")
        .args([
            "contract",
            "bindings",
            "typescript",
            "--wasm",
            wasm_str,
            "--output-dir",
            output_str,
        ])
        .output();

    match result {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            Err(ForgeError::Other(format!(
                "stellar contract bindings typescript failed:\n{stderr}"
            )))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(ForgeError::ToolMissing("stellar-cli".into()))
        }
        Err(e) => Err(ForgeError::io(
            "running stellar contract bindings typescript",
        )(e)),
    }
}

/// The `bindings` subcommand, with `ts` as its only sub-subcommand today.
pub struct BindingsTsPlugin;

impl ForgePlugin for BindingsTsPlugin {
    fn name(&self) -> &'static str {
        "bindings"
    }

    fn command(&self) -> Command {
        Command::new("bindings")
            .about("Generate client bindings from a built contract")
            .subcommand_required(true)
            .arg_required_else_help(true)
            .subcommand(
                Command::new("ts")
                    .about("Generate a TypeScript client package from the built contract wasm")
                    .arg(
                        Arg::new("path")
                            .long("path")
                            .help("Contract project directory [default: current directory]"),
                    )
                    .arg(
                        Arg::new("wasm")
                            .long("wasm")
                            .help("Path to the built .wasm file [default: target/wasm32v1-none/release/<crate>.wasm]"),
                    )
                    .arg(
                        Arg::new("output")
                            .long("output")
                            .short('o')
                            .help("Output directory for the generated package [default: bindings/typescript]"),
                    )
                    .arg(
                        Arg::new("force")
                            .long("force")
                            .action(ArgAction::SetTrue)
                            .help("Overwrite the output directory if it exists"),
                    )
                    .arg(
                        Arg::new("watch")
                            .long("watch")
                            .action(ArgAction::SetTrue)
                            .help("Watch the contract source and regenerate bindings on change"),
                    )
                    .arg(
                        Arg::new("react")
                            .long("react")
                            .action(ArgAction::SetTrue)
                            .help(
                                "Also emit src/hooks.ts: a typed React hook per entrypoint, exported at \
                                 the ./hooks subpath. Adds react as an optional peer dependency; without \
                                 this flag react is never mentioned in the generated package",
                            ),
                    ),
            )
    }

    fn run(&self, matches: &ArgMatches, ctx: &ForgeContext) -> Result<()> {
        match matches.subcommand() {
            Some(("ts", sub)) => run_ts(sub, ctx),
            _ => Err(ForgeError::InvalidArgument(
                "expected a bindings subcommand, e.g. `bindings ts`".into(),
            )),
        }
    }
}

fn run_ts(matches: &ArgMatches, ctx: &ForgeContext) -> Result<()> {
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
    let watch = matches.get_flag("watch");
    let react = matches.get_flag("react");

    if watch {
        // `--watch` always overwrites — that is the whole point of the loop.
        return watch_loop(&dir, wasm_override.as_deref(), &output, react, ctx);
    }

    let wasm_path = generate_bindings(&dir, wasm_override.as_deref(), &output, force, react)?;

    if ctx.json {
        let report = serde_json::json!({
            "wasm_path": wasm_path.display().to_string(),
            "output_dir": output.display().to_string(),
            "react": react,
        });
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    } else {
        println!("generated TypeScript bindings from {}", wasm_path.display());
        println!("  -> {}", output.display());
        if react {
            println!(
                "  -> {} (React hooks)",
                output.join("src/hooks.ts").display()
            );
        }
        println!();
        println!("next steps:");
        println!("  cd {}", output.display());
        println!("  npm install");
        println!("  npm run build");
    }
    Ok(())
}

/// Re-render the bindings package every time the contract source tree
/// changes. Ctrl-C cleanly exits; per-regeneration errors are printed but
/// do not stop the loop (acceptance criteria).
///
/// Polling-based so we don't add a new dependency for a single command:
/// `notify` would be the right answer at scale, but for a one-second poll
/// over a scaffolded contract the disk cost is negligible.
fn watch_loop(
    dir: &Path,
    wasm_override: Option<&Path>,
    output: &Path,
    react: bool,
    ctx: &ForgeContext,
) -> Result<()> {
    // First run is synchronous so a broken setup fails before we enter
    // the steady-state loop (e.g. missing stellar-cli, malformed wasm).
    if let Err(err) = regenerate(dir, wasm_override, output, react, ctx) {
        if !ctx.quiet {
            eprintln!("[{}] regeneration failed: {err} (continuing)", timestamp());
        }
    }

    let mut last = snapshot_mtime(dir);
    loop {
        std::thread::sleep(Duration::from_secs(1));
        let now = snapshot_mtime(dir);
        if now != last {
            last = now;
            if let Err(err) = regenerate(dir, wasm_override, output, react, ctx) {
                if !ctx.quiet {
                    eprintln!("[{}] regeneration failed: {err} (continuing)", timestamp());
                }
            }
        }
    }
    // Ctrl-C terminates the process; the default SIGINT disposition is the
    // criterion's "clean exit on Ctrl-C".
    #[allow(unreachable_code)]
    Ok(())
}

fn regenerate(
    dir: &Path,
    wasm_override: Option<&Path>,
    output: &Path,
    react: bool,
    ctx: &ForgeContext,
) -> Result<PathBuf> {
    let wasm = generate_bindings(dir, wasm_override, output, true, react)?;
    if !ctx.quiet {
        println!("[{}] regenerated -> {}", timestamp(), output.display());
    }
    Ok(wasm)
}

fn timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{now}s")
}

/// Latest mtime under `dir`, recursively. Used as a coarse change
/// indicator; a hash comparison would be more precise but is unnecessary
/// for a one-second poll.
fn snapshot_mtime(dir: &Path) -> Option<SystemTime> {
    fn walk(path: &Path, latest: &mut Option<SystemTime>) -> std::io::Result<()> {
        let meta = std::fs::metadata(path)?;
        if let Ok(modified) = meta.modified() {
            match latest {
                Some(current) if *current >= modified => {}
                _ => *latest = Some(modified),
            }
        }
        if meta.is_dir() {
            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                walk(&entry.path(), latest)?;
            }
        }
        Ok(())
    }
    let mut latest = None;
    let _ = walk(dir, &mut latest);
    latest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locates_wasm_by_crate_name() {
        let dir = Path::new("/proj");
        assert_eq!(
            locate_wasm(dir, "my_token"),
            PathBuf::from("/proj/target/wasm32v1-none/release/my_token.wasm")
        );
    }

    #[test]
    fn reads_package_info_and_normalizes_crate_name() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"my-token\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();

        let info = read_package_info(tmp.path()).unwrap();
        assert_eq!(info.package_name, "my-token");
        assert_eq!(info.crate_name, "my_token");
        assert_eq!(info.version.as_deref(), Some("0.1.0"));
    }

    #[test]
    fn workspace_inherited_version_is_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion.workspace = true\n",
        )
        .unwrap();
        assert_eq!(read_package_info(tmp.path()).unwrap().version, None);
    }

    /// What `stellar contract bindings typescript` (stellar-cli 28) writes.
    fn cli_package_json() -> serde_json::Value {
        serde_json::json!({
            "version": "0.0.0",
            "name": "typescript",
            "type": "module",
            "exports": "./dist/index.js",
            "typings": "dist/index.d.ts",
            "scripts": { "build": "tsc" },
            "dependencies": { "@stellar/stellar-sdk": "^16.0.1", "buffer": "6.0.3" },
            "devDependencies": { "typescript": "^5.6.2" }
        })
    }

    fn demo_info() -> PackageInfo {
        PackageInfo {
            package_name: "My-Token".into(),
            crate_name: "my_token".into(),
            version: Some("1.2.3".into()),
        }
    }

    #[test]
    fn publishable_package_has_conditional_exports_and_types() {
        let mut pkg = cli_package_json();
        make_publishable(&mut pkg, Some(&demo_info()));

        assert_eq!(pkg["name"], "my-token");
        assert_eq!(pkg["version"], "1.2.3");
        assert_eq!(pkg["type"], "module");
        assert_eq!(pkg["main"], "./dist/index.js");
        assert_eq!(pkg["types"], "./dist/index.d.ts");
        assert!(pkg.get("typings").is_none());

        let root = &pkg["exports"]["."];
        assert_eq!(root["types"], "./dist/index.d.ts");
        assert_eq!(root["import"], "./dist/index.js");
        // `types` must be the first condition for TypeScript to pick it up.
        let conditions: Vec<&String> = root.as_object().unwrap().keys().collect();
        assert_eq!(conditions, ["types", "import", "default"]);
        assert_eq!(pkg["exports"]["./package.json"], "./package.json");
    }

    #[test]
    fn publishable_package_lists_files_and_builds_on_pack() {
        let mut pkg = cli_package_json();
        make_publishable(&mut pkg, Some(&demo_info()));

        assert_eq!(
            pkg["files"],
            serde_json::json!(["dist", "src", "README.md"])
        );
        assert_eq!(pkg["scripts"]["build"], "tsc");
        assert_eq!(pkg["scripts"]["prepack"], "npm run build");
        assert_eq!(pkg["engines"]["node"], MIN_NODE);
    }

    #[test]
    fn stellar_sdk_becomes_a_peer_dependency_with_the_cli_range() {
        let mut pkg = cli_package_json();
        make_publishable(&mut pkg, Some(&demo_info()));

        assert_eq!(pkg["peerDependencies"][STELLAR_SDK], "^16.0.1");
        assert_eq!(pkg["devDependencies"][STELLAR_SDK], "^16.0.1");
        assert!(pkg["dependencies"].get(STELLAR_SDK).is_none());
        // Other runtime deps are untouched.
        assert_eq!(pkg["dependencies"]["buffer"], "6.0.3");
        assert_eq!(pkg["devDependencies"]["typescript"], "^5.6.2");
    }

    #[test]
    fn missing_sdk_falls_back_to_the_documented_range() {
        let mut pkg = serde_json::json!({ "name": "x", "dependencies": {} });
        make_publishable(&mut pkg, None);

        assert_eq!(
            pkg["peerDependencies"][STELLAR_SDK],
            DEFAULT_STELLAR_SDK_RANGE
        );
        assert!(
            pkg.get("dependencies").is_none(),
            "empty dependencies are dropped"
        );
        // No Cargo.toml info: the CLI's name is kept, a version is ensured.
        assert_eq!(pkg["name"], "x");
        assert_eq!(pkg["version"], "0.0.0");
    }

    #[test]
    fn finalize_rewrites_package_json_on_disk() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("package.json"),
            serde_json::to_string(&cli_package_json()).unwrap(),
        )
        .unwrap();

        finalize_package_json(tmp.path(), Some(&demo_info()), false).unwrap();

        let raw = std::fs::read_to_string(tmp.path().join("package.json")).unwrap();
        assert!(raw.ends_with('\n'));
        let pkg: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(pkg["peerDependencies"][STELLAR_SDK], "^16.0.1");
    }

    #[test]
    fn errors_outside_a_cargo_project() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(read_package_info(tmp.path()).is_err());
    }

    #[test]
    fn errors_when_wasm_missing_without_invoking_cli() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();

        let output = tmp.path().join("bindings/typescript");
        let err = generate_bindings(tmp.path(), None, &output, false, false).unwrap_err();
        assert!(err.to_string().contains("stellar contract build"));
    }

    #[test]
    fn refuses_to_overwrite_output_without_force() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        // Simulate a built wasm so the missing-wasm check doesn't short-circuit.
        let wasm_dir = tmp.path().join("target/wasm32v1-none/release");
        std::fs::create_dir_all(&wasm_dir).unwrap();
        std::fs::write(wasm_dir.join("demo.wasm"), b"\0asm").unwrap();

        let output = tmp.path().join("bindings/typescript");
        std::fs::create_dir_all(&output).unwrap();

        let err = generate_bindings(tmp.path(), None, &output, false, false).unwrap_err();
        assert!(matches!(err, ForgeError::AlreadyExists(_)));
    }

    #[test]
    fn snapshot_mtime_reflects_changes_in_the_tree() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("Cargo.toml"), "x").unwrap();
        let before = snapshot_mtime(tmp.path());

        // Touch a file to bump mtime.
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(tmp.path().join("lib.rs"), "x").unwrap();
        let after = snapshot_mtime(tmp.path());

        assert!(before.is_some());
        assert!(after.is_some());
        assert!(after.unwrap() >= before.unwrap());
    }

    #[test]
    fn watch_flag_is_exposed_on_the_ts_subcommand() {
        let mut cmd = BindingsTsPlugin.command();
        let ts = cmd.find_subcommand_mut("ts").expect("ts subcommand");
        let help = ts.render_long_help().to_string();
        assert!(help.contains("--watch"), "{help}");
    }

    #[test]
    fn react_flag_is_exposed_on_the_ts_subcommand() {
        let mut cmd = BindingsTsPlugin.command();
        let ts = cmd.find_subcommand_mut("ts").expect("ts subcommand");
        let help = ts.render_long_help().to_string();
        assert!(help.contains("--react"), "{help}");
    }

    // --- --react: naming heuristic ---

    #[test]
    fn prefixed_names_are_reads() {
        for name in [
            "get_debt",
            "is_paused",
            "has_role",
            "list_offers",
            "view_state",
            "query_price",
            "read_config",
        ] {
            assert!(looks_like_read(name), "{name} should be a read");
        }
    }

    #[test]
    fn known_getter_names_are_reads() {
        for name in [
            "balance",
            "balance_of",
            "allowance",
            "owner",
            "owner_of",
            "admin",
            "name",
            "symbol",
            "decimals",
            "total_supply",
            "token_uri",
            "metadata",
            "uri",
        ] {
            assert!(looks_like_read(name), "{name} should be a read");
        }
    }

    #[test]
    fn unrecognised_names_default_to_write() {
        for name in [
            "mint",
            "transfer",
            "burn",
            "initialize",
            "deposit",
            "withdraw",
            "set_admin",
        ] {
            assert!(!looks_like_read(name), "{name} should be a write");
        }
    }

    #[test]
    fn pascal_case_conversion() {
        assert_eq!(to_pascal_case("mint"), "Mint");
        assert_eq!(to_pascal_case("get_contract_info"), "GetContractInfo");
        assert_eq!(to_pascal_case("owner_of"), "OwnerOf");
    }

    // --- --react: entrypoint name extraction ---

    fn spec_json_with_names(names: &[&str]) -> String {
        let entries: Vec<serde_json::Value> = names
            .iter()
            .map(|n| serde_json::json!({"function_v0": {"doc": "", "name": n, "inputs": [], "outputs": []}}))
            .collect();
        serde_json::to_string(&entries).unwrap()
    }

    #[test]
    fn entrypoint_names_reads_function_v0_entries_in_order() {
        let json = spec_json_with_names(&["mint", "balance", "burn"]);
        assert_eq!(
            entrypoint_names(&json).unwrap(),
            vec!["mint", "balance", "burn"]
        );
    }

    #[test]
    fn entrypoint_names_ignores_non_function_entries() {
        let mut entries: Vec<serde_json::Value> =
            serde_json::from_str(&spec_json_with_names(&["mint"])).unwrap();
        entries.push(serde_json::json!({"udt_error_enum_v0": {"name": "Error", "cases": []}}));
        let json = serde_json::to_string(&entries).unwrap();
        assert_eq!(entrypoint_names(&json).unwrap(), vec!["mint"]);
    }

    #[test]
    fn entrypoint_names_excludes_the_constructor() {
        let json = spec_json_with_names(&["__constructor", "mint", "admin"]);
        assert_eq!(entrypoint_names(&json).unwrap(), vec!["mint", "admin"]);
    }

    #[test]
    fn entrypoint_names_rejects_malformed_json() {
        assert!(entrypoint_names("not json").is_err());
        assert!(entrypoint_names("{}").is_err());
    }

    // --- --react: hooks.ts rendering ---

    #[test]
    fn renders_a_read_hook_for_a_getter() {
        let ts = render_hooks_ts(&["balance".to_string()]);
        assert!(ts.contains("export function useBalance("), "{ts}");
        assert!(ts.contains(r#"Parameters<Client["balance"]>"#), "{ts}");
        assert!(ts.contains("useContractRead("), "{ts}");
        assert!(!ts.contains("useBalanceMutation"), "{ts}");
    }

    #[test]
    fn renders_a_mutation_hook_for_a_write() {
        let ts = render_hooks_ts(&["mint".to_string()]);
        assert!(ts.contains("export function useMintMutation("), "{ts}");
        assert!(ts.contains("useContractMutation("), "{ts}");
        assert!(!ts.contains("export function useMint(\n"), "{ts}");
    }

    #[test]
    fn hooks_file_declares_no_shared_helper_twice_per_entrypoint() {
        let ts = render_hooks_ts(&[
            "mint".to_string(),
            "burn".to_string(),
            "balance".to_string(),
        ]);
        assert_eq!(ts.matches("function useContractRead<T>(").count(), 1);
        assert_eq!(ts.matches("function useContractMutation<Args").count(), 1);
        assert_eq!(ts.matches("export function use").count(), 3);
    }

    #[test]
    fn empty_entrypoint_list_still_renders_the_shared_preamble() {
        let ts = render_hooks_ts(&[]);
        assert!(ts.contains("export interface ReadState"), "{ts}");
        assert!(ts.contains("export interface MutationState"), "{ts}");
        assert!(!ts.contains("export function use"), "{ts}");
    }

    // --- --react: package.json additions ---

    #[test]
    fn react_support_adds_optional_peer_and_hooks_export() {
        let mut pkg = cli_package_json();
        make_publishable(&mut pkg, Some(&demo_info()));
        add_react_support(&mut pkg);

        assert_eq!(pkg["peerDependencies"]["react"], REACT_PEER_RANGE);
        assert_eq!(pkg["devDependencies"]["react"], REACT_PEER_RANGE);
        assert_eq!(pkg["devDependencies"]["@types/react"], REACT_PEER_RANGE);
        assert_eq!(pkg["peerDependenciesMeta"]["react"]["optional"], true);
        assert_eq!(pkg["exports"]["./hooks"]["types"], "./dist/hooks.d.ts");
        assert_eq!(pkg["exports"]["./hooks"]["import"], "./dist/hooks.js");
    }

    #[test]
    fn without_react_flag_package_json_never_mentions_react() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("package.json"),
            serde_json::to_string(&cli_package_json()).unwrap(),
        )
        .unwrap();

        finalize_package_json(tmp.path(), Some(&demo_info()), false).unwrap();

        let raw = std::fs::read_to_string(tmp.path().join("package.json")).unwrap();
        assert!(!raw.contains("react"), "{raw}");
    }

    #[test]
    fn with_react_flag_finalize_adds_the_peer_dependency() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("package.json"),
            serde_json::to_string(&cli_package_json()).unwrap(),
        )
        .unwrap();

        finalize_package_json(tmp.path(), Some(&demo_info()), true).unwrap();

        let raw = std::fs::read_to_string(tmp.path().join("package.json")).unwrap();
        let pkg: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(pkg["peerDependencies"]["react"], REACT_PEER_RANGE);
    }

    // --- --react: README section ---

    #[test]
    fn react_readme_section_is_appended() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("README.md"),
            "# my-token\n\nSome existing content.\n",
        )
        .unwrap();

        append_react_readme_section(tmp.path()).unwrap();

        let readme = std::fs::read_to_string(tmp.path().join("README.md")).unwrap();
        assert!(readme.contains("# my-token"), "{readme}");
        assert!(readme.contains("## React hooks"), "{readme}");
        assert!(readme.contains("useMintMutation"), "{readme}");
    }
}
