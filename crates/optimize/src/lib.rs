//! # soroban-forge-optimize
//!
//! `soroban-forge optimize` — wraps `stellar contract optimize` and reports
//! the wasm size before and after.
//!
//! Per soroban-forge's "wrap, don't reimplement" rule, the optimization
//! itself is delegated to the official `stellar` CLI; this module only
//! locates the local build, runs it, and reports the size delta.

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

/// Budget for `--check` from `forge.toml`.
#[derive(Deserialize)]
struct ForgeConfig {
    optimize: Option<OptimizeConfig>,
}

#[derive(Deserialize)]
struct OptimizeConfig {
    #[serde(rename = "max-size", alias = "max_size")]
    max_size: Option<u64>,
}

/// Read `[package].name` from `dir/Cargo.toml` and return it as a crate name
/// (snake_case), which is what the build output is named after.
///
/// Deliberately duplicated rather than shared with `verify`: modules depend
/// only on `soroban-forge-core`, never on each other.
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

/// Read `max-size` from `[optimize]` in `dir/forge.toml`, if present.
fn forge_config_budget(dir: &Path) -> Result<Option<u64>> {
    let config_path = dir.join("forge.toml");
    if !config_path.is_file() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&config_path)
        .map_err(ForgeError::io(format!("reading {}", config_path.display())))?;
    let config: ForgeConfig = toml::from_str(&raw).map_err(|e| ForgeError::Config {
        path: config_path.clone(),
        message: e.to_string(),
    })?;
    Ok(config.optimize.and_then(|optimize| optimize.max_size))
}

/// Default location `stellar contract build` writes its release wasm to.
pub fn locate_wasm(dir: &Path, crate_name: &str) -> PathBuf {
    dir.join("target/wasm32v1-none/release")
        .join(format!("{crate_name}.wasm"))
}

/// Resolve which local wasm to optimize: `wasm_override` when given,
/// otherwise the release build of the cargo project in `dir`.
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

/// Path `stellar contract optimize` writes its output to for a given input.
pub fn optimized_wasm_path(wasm_path: &Path) -> PathBuf {
    let stem = wasm_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    wasm_path.with_file_name(format!("{stem}.optimized.wasm"))
}

fn path_str(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| ForgeError::Other(format!("path {} is not valid UTF-8", path.display())))
}

/// Run `stellar contract optimize --wasm <wasm_path>`, streaming its output
/// directly to the terminal. Never reimplemented locally.
///
/// Thin system-touching wrapper; not unit-tested.
fn run_stellar_optimize(wasm_path: &Path) -> Result<()> {
    let wasm_str = path_str(wasm_path)?;

    let mut cmd = std::process::Command::new("stellar");
    cmd.args(["contract", "optimize", "--wasm", wasm_str]);
    log::debug!("optimizing {wasm_str}");

    match cmd.status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(ForgeError::Other(format!(
            "stellar contract optimize failed (exit {status})"
        ))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(ForgeError::ToolMissing("stellar-cli".into()))
        }
        Err(e) => Err(ForgeError::io("running stellar contract optimize")(e)),
    }
}

/// The outcome of one optimize run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OptimizeReport {
    pub wasm: String,
    pub optimized_wasm: String,
    pub before_bytes: u64,
    pub after_bytes: u64,
    /// Positive when the optimized wasm is smaller.
    pub saved_bytes: i64,
}

impl OptimizeReport {
    pub fn new(wasm: &Path, optimized_wasm: &Path, before_bytes: u64, after_bytes: u64) -> Self {
        Self {
            wasm: wasm.display().to_string(),
            optimized_wasm: optimized_wasm.display().to_string(),
            before_bytes,
            after_bytes,
            saved_bytes: before_bytes as i64 - after_bytes as i64,
        }
    }

    /// Percentage reduction from `before_bytes` to `after_bytes`, `0.0` when
    /// `before_bytes` is `0`.
    pub fn percent_saved(&self) -> f64 {
        if self.before_bytes == 0 {
            0.0
        } else {
            (self.saved_bytes as f64 / self.before_bytes as f64) * 100.0
        }
    }
}

/// Optimize the wasm at `wasm_path` in place via `stellar contract optimize`
/// and report the size delta.
pub fn optimize(wasm_path: &Path) -> Result<OptimizeReport> {
    let before_bytes = std::fs::metadata(wasm_path)
        .map_err(ForgeError::io(format!("reading {}", wasm_path.display())))?
        .len();

    run_stellar_optimize(wasm_path)?;

    let optimized_path = optimized_wasm_path(wasm_path);
    let after_bytes = std::fs::metadata(&optimized_path)
        .map_err(ForgeError::io(format!(
            "reading {}",
            optimized_path.display()
        )))?
        .len();

    Ok(OptimizeReport::new(
        wasm_path,
        &optimized_path,
        before_bytes,
        after_bytes,
    ))
}

/// Human-readable report, printed unless `--quiet`.
pub fn format_report(report: &OptimizeReport) -> String {
    format!(
        "optimized {} -> {}\n\n  before  {} bytes\n  after   {} bytes\n  saved   {} bytes ({:.1}%)\n",
        report.wasm,
        report.optimized_wasm,
        report.before_bytes,
        report.after_bytes,
        report.saved_bytes,
        report.percent_saved(),
    )
}

/// The same report as JSON, for `--json`.
pub fn json_report(report: &OptimizeReport) -> String {
    serde_json::to_string_pretty(report).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

/// Fail if the optimized size exceeds the configured budget.
pub fn check_budget(report: &OptimizeReport, max_size: Option<u64>) -> Result<()> {
    if let Some(limit) = max_size {
        if report.after_bytes > limit {
            return Err(ForgeError::Other(format!(
                "optimized wasm size {} bytes exceeds budget of {} bytes",
                report.after_bytes, limit
            )));
        }
    }
    Ok(())
}

/// The `optimize` subcommand.
pub struct OptimizePlugin;

impl ForgePlugin for OptimizePlugin {
    fn name(&self) -> &'static str {
        "optimize"
    }

    fn command(&self) -> Command {
        Command::new("optimize")
            .about("Optimize the release wasm and report the before/after size")
            .long_about(
                "Run `stellar contract optimize` against the local release build and \
                 report how much smaller the optimized wasm is.",
            )
            .arg(
                Arg::new("path")
                    .long("path")
                    .help("Contract project directory [default: current directory]"),
            )
            .arg(
                Arg::new("wasm")
                    .long("wasm")
                    .help("Path to the local .wasm to optimize [default: target/wasm32v1-none/release/<crate>.wasm]"),
            )
            .arg(
                Arg::new("check")
                    .long("check")
                    .help("Fail if the optimized wasm exceeds --max-size or the forge.toml budget")
                    .action(clap::ArgAction::SetTrue),
            )
            .arg(
                Arg::new("max_size")
                    .long("max-size")
                    .value_name("BYTES")
                    .value_parser(clap::value_parser!(u64))
                    .help("Maximum allowed size in bytes after optimization"),
            )
    }

    fn run(&self, matches: &ArgMatches, ctx: &ForgeContext) -> Result<()> {
        let dir = matches
            .get_one::<String>("path")
            .map(|p| ctx.cwd.join(p))
            .unwrap_or_else(|| ctx.cwd.clone());
        let wasm_override = matches.get_one::<String>("wasm").map(|p| ctx.cwd.join(p));

        let wasm_path = resolve_local_wasm(&dir, wasm_override.as_deref())?;

        let check = matches.get_flag("check");
        let max_size = if check {
            if let Some(cli) = matches.get_one::<u64>("max_size").copied() {
                Some(cli)
            } else {
                match forge_config_budget(&dir)? {
                    Some(config) => Some(config),
                    None => {
                        return Err(ForgeError::Other(
                            "--check requires --max-size or a forge.toml [optimize] max-size".into(),
                        ));
                    }
                }
            }
        } else {
            None
        };

        let report = optimize(&wasm_path)?;
        check_budget(&report, max_size)?;

        if ctx.json {
            println!("{}", json_report(&report));
        } else if !ctx.quiet {
            print!("{}", format_report(&report));
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
        std::fs::write(&custom, b"\0asm").unwrap();

        assert_eq!(
            resolve_local_wasm(tmp.path(), Some(&custom)).unwrap(),
            custom
        );
    }

    #[test]
    fn derives_the_optimized_output_path() {
        assert_eq!(
            optimized_wasm_path(Path::new("/proj/target/release/my_token.wasm")),
            PathBuf::from("/proj/target/release/my_token.optimized.wasm")
        );
    }

    #[test]
    fn report_computes_saved_bytes_and_percent() {
        let report = OptimizeReport::new(
            Path::new("a.wasm"),
            Path::new("a.optimized.wasm"),
            1000,
            750,
        );
        assert_eq!(report.saved_bytes, 250);
        assert!((report.percent_saved() - 25.0).abs() < f64::EPSILON);
    }

    #[test]
    fn report_handles_no_reduction() {
        let report = OptimizeReport::new(
            Path::new("a.wasm"),
            Path::new("a.optimized.wasm"),
            1000,
            1000,
        );
        assert_eq!(report.saved_bytes, 0);
        assert_eq!(report.percent_saved(), 0.0);
    }

    #[test]
    fn format_report_includes_sizes() {
        let report = OptimizeReport::new(
            Path::new("a.wasm"),
            Path::new("a.optimized.wasm"),
            1000,
            750,
        );
        let text = format_report(&report);
        assert!(text.contains("1000 bytes"), "{text}");
        assert!(text.contains("750 bytes"), "{text}");
        assert!(text.contains("250 bytes"), "{text}");
    }

    #[test]
    fn json_report_carries_all_fields() {
        let report = OptimizeReport::new(
            Path::new("a.wasm"),
            Path::new("a.optimized.wasm"),
            1000,
            750,
        );
        let parsed: serde_json::Value = serde_json::from_str(&json_report(&report)).unwrap();
        assert_eq!(parsed["before_bytes"], 1000);
        assert_eq!(parsed["after_bytes"], 750);
        assert_eq!(parsed["saved_bytes"], 250);
    }

    #[test]
    fn plugin_name_matches_its_command() {
        let plugin = OptimizePlugin;
        assert_eq!(plugin.name(), plugin.command().get_name());
    }

    #[test]
    fn help_documents_path_and_wasm_flags() {
        let help = OptimizePlugin.command().render_long_help().to_string();
        assert!(help.contains("--path"), "{help}");
        assert!(help.contains("--wasm"), "{help}");
    }
}
