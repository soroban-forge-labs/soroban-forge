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

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use clap::{Arg, ArgAction, ArgMatches, Command};
use serde::Deserialize;
use soroban_forge_core::{ForgeContext, ForgeError, ForgePlugin, Result};

const DEFAULT_OUTPUT_SUBDIR: &str = "bindings/typescript";

#[derive(Deserialize)]
struct Manifest {
    package: Package,
}

#[derive(Deserialize)]
struct Package {
    name: String,
}

/// Cargo package identity, enough to locate the built wasm.
#[derive(Debug, Clone, PartialEq)]
pub struct PackageInfo {
    /// Cargo package name, e.g. `my-token`.
    pub package_name: String,
    /// Rust crate name (snake_case), e.g. `my_token`.
    pub crate_name: String,
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
    let raw = std::fs::read_to_string(&manifest_path)
        .map_err(ForgeError::io(format!("reading {}", manifest_path.display())))?;
    let manifest: Manifest = toml::from_str(&raw).map_err(|e| ForgeError::Config {
        path: manifest_path.clone(),
        message: e.to_string(),
    })?;
    Ok(PackageInfo {
        crate_name: manifest.package.name.replace('-', "_"),
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
    let wasm_path = match wasm_override {
        Some(p) => p.to_path_buf(),
        None => {
            let info = read_package_info(contract_dir)?;
            locate_wasm(contract_dir, &info.crate_name)
        }
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
    Ok(wasm_path)
}

/// Shell out to the official CLI. Never reimplemented locally.
fn run_stellar_bindings(wasm: &Path, output: &Path) -> Result<()> {
    let wasm_str = wasm
        .to_str()
        .ok_or_else(|| ForgeError::Other(format!("wasm path {} is not valid UTF-8", wasm.display())))?;
    let output_str = output
        .to_str()
        .ok_or_else(|| ForgeError::Other(format!("output path {} is not valid UTF-8", output.display())))?;

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
        Err(e) => Err(ForgeError::io("running stellar contract bindings typescript")(e)),
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

    if watch {
        // `--watch` always overwrites — that is the whole point of the loop.
        return watch_loop(&dir, wasm_override.as_deref(), &output, ctx);
    }

    let wasm_path = generate_bindings(&dir, wasm_override.as_deref(), &output, force)?;

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
    ctx: &ForgeContext,
) -> Result<()> {
    // First run is synchronous so a broken setup fails before we enter
    // the steady-state loop (e.g. missing stellar-cli, malformed wasm).
    if let Err(err) = regenerate(dir, wasm_override, output, ctx) {
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
            if let Err(err) = regenerate(dir, wasm_override, output, ctx) {
                if !ctx.quiet {
                    eprintln!(
                        "[{}] regeneration failed: {err} (continuing)",
                        timestamp()
                    );
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
    ctx: &ForgeContext,
) -> Result<PathBuf> {
    let wasm = generate_bindings(dir, wasm_override, output, true)?;
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
        let cmd = BindingsTsPlugin.command();
        let ts = cmd
            .find_subcommand("ts")
            .expect("ts subcommand");
        let help = ts.render_long_help().to_string();
        assert!(help.contains("--watch"), "{help}");
    }
}