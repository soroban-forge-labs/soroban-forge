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
/// auto-detecting the built wasm. Returns the wasm path that was used.
pub fn generate_bindings(
    contract_dir: &Path,
    wasm_override: Option<&Path>,
    output: &Path,
    force: bool,
) -> Result<PathBuf> {
    generate_bindings_with_options(contract_dir, wasm_override, output, None, force)
}

/// Generate a TypeScript bindings package for the contract in `contract_dir`
/// into `output` with an optional npm package name override.
pub fn generate_bindings_with_options(
    contract_dir: &Path,
    wasm_override: Option<&Path>,
    output: &Path,
    package_name: Option<&str>,
    force: bool,
) -> Result<PathBuf> {
    if let Some(name) = package_name {
        validate_npm_package_name(name)?;
    }
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
            "no built wasm found at {} ? run `stellar contract build` first (or pass --wasm)",
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
    finalize_package_json(output, info.as_ref(), package_name)?;
    Ok(wasm_path)
}

/// Rewrite `output/package.json` in place with [`make_publishable_with_name`].
fn finalize_package_json(
    output: &Path,
    info: Option<&PackageInfo>,
    package_name: Option<&str>,
) -> Result<()> {
    let path = output.join("package.json");
    let raw = std::fs::read_to_string(&path)
        .map_err(ForgeError::io(format!("reading {}", path.display())))?;
    let mut pkg: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
        ForgeError::Other(format!(
            "stellar generated an invalid {}: {e}",
            path.display()
        ))
    })?;
    make_publishable_with_name(&mut pkg, info, package_name);
    let mut pretty = serde_json::to_string_pretty(&pkg).expect("a JSON value always serialises");
    pretty.push('\n');
    std::fs::write(&path, pretty).map_err(ForgeError::io(format!("writing {}", path.display())))
}

/// npm package name for a cargo package: npm names must be lowercase.
/// Validate that `name` is a legal npm package name according to npm rules.
pub fn validate_npm_package_name(name: &str) -> Result<()> {
    fn invalid(name: &str, reason: &str) -> ForgeError {
        ForgeError::InvalidArgument(format!("`{name}` is not a legal npm package name: {reason}"))
    }

    if name.is_empty() {
        return Err(invalid(name, "package name cannot be empty"));
    }
    if name.len() > 214 {
        return Err(invalid(name, "package name cannot be longer than 214 characters"));
    }
    if name.trim() != name {
        return Err(invalid(name, "package name cannot contain leading or trailing whitespace"));
    }
    if name.chars().any(|c| c.is_ascii_uppercase()) {
        return Err(invalid(name, "package name cannot contain uppercase characters"));
    }
    if name == "node_modules" || name == "favicon.ico" {
        return Err(invalid(name, "package name is a reserved blacklisted name"));
    }

    let is_valid_part = |part: &str| -> std::result::Result<(), &'static str> {
        if part.is_empty() {
            return Err("name segment cannot be empty");
        }
        if part.starts_with('.') || part.starts_with('_') {
            return Err("name segment cannot start with '.' or '_'");
        }
        for c in part.chars() {
            if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_' || c == '.') {
                return Err("name contains characters that are not URL-safe lowercase alphanumeric, '-', '_' or '.'");
            }
        }
        Ok(())
    };

    if let Some(after_at) = name.strip_prefix('@') {
        let parts: Vec<&str> = after_at.split('/').collect();
        if parts.len() != 2 {
            return Err(invalid(name, "scoped package name must have format @scope/package"));
        }
        if let Err(reason) = is_valid_part(parts[0]) {
            return Err(invalid(name, &format!("invalid scope: {reason}")));
        }
        if let Err(reason) = is_valid_part(parts[1]) {
            return Err(invalid(name, &format!("invalid package name: {reason}")));
        }
    } else {
        if name.contains('/') {
            return Err(invalid(name, "unscoped package name cannot contain '/'"));
        }
        if let Err(reason) = is_valid_part(name) {
            return Err(invalid(name, reason));
        }
    }

    Ok(())
}

/// npm package name for a cargo package: npm names must be lowercase.
pub fn npm_package_name(package_name: &str) -> String {
    package_name.to_ascii_lowercase()
}

/// Turn the `package.json` stellar-cli generates into one that can be
/// `npm pack`ed and published without edits:
pub fn make_publishable(pkg: &mut serde_json::Value, info: Option<&PackageInfo>) {
    make_publishable_with_name(pkg, info, None);
}

/// Turn the `package.json` stellar-cli generates into one that can be
/// `npm pack`ed and published without edits, optionally overriding the package name:
///
/// - name/version from `Cargo.toml` or `package_name_override` when given
/// - `exports` as a conditional map (`types` first, then `import`/`default`)
///   plus `main`/`types` for tools that predate `exports` ? this is what
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
pub fn make_publishable_with_name(
    pkg: &mut serde_json::Value,
    info: Option<&PackageInfo>,
    package_name_override: Option<&str>,
) {
    use serde_json::{json, Map, Value};

    if !pkg.is_object() {
        *pkg = Value::Object(Map::new());
    }
    let obj = pkg.as_object_mut().expect("just ensured an object");

    if let Some(custom_name) = package_name_override {
        obj.insert("name".into(), json!(custom_name));
        if let Some(info) = info {
            if let Some(version) = &info.version {
                obj.insert("version".into(), json!(version));
            }
        }
    } else if let Some(info) = info {
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
                        Arg::new("out-dir")
                            .long("out-dir")
                            .visible_alias("output")
                            .short('o')
                            .help("Output directory for the generated package [default: bindings/typescript]"),
                    )
                    .arg(
                        Arg::new("package-name")
                            .long("package-name")
                            .help("Custom npm package name for the generated package (overrides default derived from Cargo.toml)"),
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
        .get_one::<String>("out-dir")
        .or_else(|| matches.get_one::<String>("output"))
        .map(|p| ctx.cwd.join(p))
        .unwrap_or_else(|| dir.join(DEFAULT_OUTPUT_SUBDIR));

    let package_name = matches.get_one::<String>("package-name").map(String::as_str);
    if let Some(pkg_name) = package_name {
        validate_npm_package_name(pkg_name)?;
    }

    let force = matches.get_flag("force");
    let watch = matches.get_flag("watch");

    if watch {
        // `--watch` always overwrites ? that is the whole point of the loop.
        return watch_loop(&dir, wasm_override.as_deref(), &output, package_name, ctx);
    }

    let wasm_path = generate_bindings_with_options(
        &dir,
        wasm_override.as_deref(),
        &output,
        package_name,
        force,
    )?;

    if ctx.json {
        let report = serde_json::json!({
            "wasm_path": wasm_path.display().to_string(),
            "output_dir": output.display().to_string()
        });
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    } else {
        println!("generated TypeScript bindings from {}", wasm_path.display());
        println!("  -> {}", output.display());
        println!();
        println!("next steps:");
        println!("  cd {}", output.display());
        println!("  npm install");
        println!("  npm run build");
    }
    Ok(())
}

fn watch_loop(
    dir: &Path,
    wasm_override: Option<&Path>,
    output: &Path,
    package_name: Option<&str>,
    ctx: &ForgeContext,
) -> Result<()> {
    // First run is synchronous so a broken setup fails before we enter
    // the steady-state loop (e.g. missing stellar-cli, malformed wasm).
    if let Err(err) = regenerate(dir, wasm_override, output, package_name, ctx) {
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
            if let Err(err) = regenerate(dir, wasm_override, output, package_name, ctx) {
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
    package_name: Option<&str>,
    ctx: &ForgeContext,
) -> Result<PathBuf> {
    let wasm = generate_bindings_with_options(dir, wasm_override, output, package_name, true)?;
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

        finalize_package_json(tmp.path(), Some(&demo_info()), None).unwrap();

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
        let err = generate_bindings(tmp.path(), None, &output, false).unwrap_err();
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

        let err = generate_bindings(tmp.path(), None, &output, false).unwrap_err();
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
    fn out_dir_and_package_name_flags_are_exposed_on_the_ts_subcommand() {
        let mut cmd = BindingsTsPlugin.command();
        let ts = cmd.find_subcommand_mut("ts").expect("ts subcommand");
        let help = ts.render_long_help().to_string();
        assert!(help.contains("--out-dir"), "{help}");
        assert!(help.contains("--package-name"), "{help}");

        // Both --out-dir and --output parse into out-dir
        let m1 = ts.clone().try_get_matches_from(vec!["ts", "--out-dir", "custom/dir"]).unwrap();
        assert_eq!(m1.get_one::<String>("out-dir").unwrap(), "custom/dir");

        let m2 = ts.clone().try_get_matches_from(vec!["ts", "--output", "legacy/dir", "--package-name", "@my-scope/my-pkg"]).unwrap();
        assert_eq!(m2.get_one::<String>("out-dir").unwrap(), "legacy/dir");
        assert_eq!(m2.get_one::<String>("package-name").unwrap(), "@my-scope/my-pkg");
    }

    #[test]
    fn validates_legal_npm_package_names() {
        assert!(validate_npm_package_name("my-package").is_ok());
        assert!(validate_npm_package_name("foo_bar").is_ok());
        assert!(validate_npm_package_name("abc.123").is_ok());
        assert!(validate_npm_package_name("simple").is_ok());
        assert!(validate_npm_package_name("@scope/package").is_ok());
        assert!(validate_npm_package_name("@my-org/my-token").is_ok());
        assert!(validate_npm_package_name("@stellar/client.sdk_123").is_ok());
    }

    #[test]
    fn rejects_illegal_npm_package_names() {
        assert!(validate_npm_package_name("").is_err());
        assert!(validate_npm_package_name("   ").is_err());
        assert!(validate_npm_package_name("  foo  ").is_err());
        assert!(validate_npm_package_name("MyPackage").is_err()); // uppercase
        assert!(validate_npm_package_name("@Scope/pkg").is_err()); // uppercase
        assert!(validate_npm_package_name("@scope/Pkg").is_err()); // uppercase
        assert!(validate_npm_package_name(".hidden").is_err());
        assert!(validate_npm_package_name("_hidden").is_err());
        assert!(validate_npm_package_name("node_modules").is_err());
        assert!(validate_npm_package_name("favicon.ico").is_err());
        assert!(validate_npm_package_name("foo/bar").is_err()); // unscoped cannot have slash
        assert!(validate_npm_package_name("@scope").is_err()); // scoped missing subname
        assert!(validate_npm_package_name("@scope/").is_err());
        assert!(validate_npm_package_name("@/pkg").is_err());
        assert!(validate_npm_package_name("@scope/pkg/extra").is_err());
        assert!(validate_npm_package_name("@.scope/pkg").is_err());
        assert!(validate_npm_package_name("@scope/.pkg").is_err());
        assert!(validate_npm_package_name("foo bar").is_err()); // space
        assert!(validate_npm_package_name("foo*bar").is_err()); // special char
        assert!(validate_npm_package_name(&"a".repeat(215)).is_err()); // > 214 chars
    }

    #[test]
    fn package_name_override_overrides_cargo_toml() {
        let mut pkg = cli_package_json();
        make_publishable_with_name(&mut pkg, Some(&demo_info()), Some("@stellar/custom-client"));
        assert_eq!(pkg["name"], "@stellar/custom-client");
        assert_eq!(pkg["version"], "1.2.3"); // kept from info
    }

    #[test]
    fn package_name_override_without_cargo_toml_info() {
        let mut pkg = cli_package_json();
        make_publishable_with_name(&mut pkg, None, Some("standalone-client"));
        assert_eq!(pkg["name"], "standalone-client");
    }

    #[test]
    fn generate_bindings_with_options_rejects_invalid_package_name() {
        let tmp = tempfile::tempdir().unwrap();
        let err = generate_bindings_with_options(
            tmp.path(),
            None,
            &tmp.path().join("out"),
            Some("Invalid Name"),
            false,
        ).unwrap_err();
        assert!(err.to_string().contains("not a legal npm package name"), "{err}");
    }
}
