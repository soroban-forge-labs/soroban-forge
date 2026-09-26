//! # soroban-forge-optimize
//!
//! `soroban-forge optimize` — wraps `stellar contract optimize` and reports
//! the wasm size before and after.
//!
//! Per soroban-forge's "wrap, don't reimplement" rule, the optimization
//! itself is delegated to the official `stellar` CLI; this module only
//! locates the local build, runs it, and reports the size delta.
//!
//! Before shelling out, `optimize` pre-flights the installed `stellar-cli`
//! (presence, minimum version, known-broken releases), reusing the
//! `soroban-forge-core::toolchain` helpers that `doctor` uses.  This turns a
//! cryptic "unknown subcommand: contract" into a clear error pointing at
//! `soroban-forge doctor`.

use std::path::{Path, PathBuf};

use clap::{Arg, ArgMatches, Command};
use serde::{Deserialize, Serialize};
use soroban_forge_core::toolchain::{
    known_broken_stellar_cli_replacement, version_at_least, MIN_STELLAR,
};
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

/// Sibling temp path used to stage the optimized bytes before an atomic
/// rename over `original`.  Pid-scoped so concurrent `optimize --in-place`
/// runs against the same directory cannot clobber each other's staging file.
fn staging_path(original: &Path) -> PathBuf {
    let stem = original
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let pid = std::process::id();
    original.with_file_name(format!("{stem}.{pid}.tmp"))
}

/// Move the bytes at `optimized` over `original`, then remove `optimized`.
///
/// Used by `optimize --in-place` so a deploy pipeline can keep a single
/// canonical artifact path.
///
/// Failure safety: the optimized bytes are staged in a sibling temp file and
/// then `rename`d over `original`.  On every supported platform `rename` on
/// the same filesystem is atomic, so if any step before the rename fails the
/// original file is left byte-for-byte untouched — satisfying the "original
/// preserved on failure" guarantee.  A staging file left behind by a failed
/// rename is cleaned up on the error path.
pub fn replace_with_optimized(original: &Path, optimized: &Path) -> Result<()> {
    let bytes = std::fs::read(optimized)
        .map_err(ForgeError::io(format!("reading {}", optimized.display())))?;

    let staging = staging_path(original);
    // Guard so the staging file never leaks on an error path.
    let cleanup = StagingGuard(&staging);

    std::fs::write(&staging, &bytes).map_err(ForgeError::io(format!(
        "writing staging file {}",
        staging.display()
    )))?;

    std::fs::rename(&staging, original).map_err(ForgeError::io(format!(
        "replacing {} with optimized bytes",
        original.display()
    )))?;
    cleanup.disarm();

    std::fs::remove_file(optimized)
        .map_err(ForgeError::io(format!("removing {}", optimized.display())))?;
    Ok(())
}

/// Deletes its path on drop unless disarmed.  A tiny RAII helper so the
/// staging file cannot linger if an intermediate step fails.
struct StagingGuard<'a>(&'a Path);

impl StagingGuard<'_> {
    fn disarm(self) {
        std::mem::forget(self);
    }
}

impl Drop for StagingGuard<'_> {
    fn drop(&mut self) {
        // Best-effort: if the file was already renamed away, `remove_file`
        // returns NotFound and we ignore it.  If cleanup itself fails we
        // also ignore it — the original file is what matters.
        let _ = std::fs::remove_file(self.0);
    }
}

/// Pre-flight gate on the installed `stellar-cli`, evaluated before we
/// shell out to it (issue #415).
///
/// `version_line` is the first line of `stellar --version`, or `None` when
/// the binary could not be run at all.  Pure and side-effect-free so it can
/// be unit-tested without a `stellar` binary on `PATH`:
///
/// - `None` -> [`ForgeError::ToolMissing`] (binary absent or unlaunchable)
/// - version < [`MIN_STELLAR`] -> [`ForgeError::ToolUnsupported`]
/// - version in the known-broken denylist -> [`ForgeError::ToolUnsupported`]
/// - otherwise -> `Ok(())`
///
/// Both error variants render a pointer at `soroban-forge doctor`, which is
/// what the acceptance criterion asks for.
pub fn check_stellar_cli(version_line: Option<&str>) -> Result<()> {
    let Some(line) = version_line else {
        return Err(ForgeError::ToolMissing("stellar-cli".into()));
    };

    if !version_at_least(line, MIN_STELLAR) {
        return Err(ForgeError::ToolUnsupported(format!(
            "stellar-cli {line} is too old for `optimize` (need >= {}.{})",
            MIN_STELLAR.0, MIN_STELLAR.1
        )));
    }

    if let Some(recommended) = known_broken_stellar_cli_replacement(line) {
        return Err(ForgeError::ToolUnsupported(format!(
            "stellar-cli {line} is a known-broken release (upgrade to {recommended})"
        )));
    }

    Ok(())
}

/// Probe the installed `stellar-cli` version line.
///
/// Thin system-touching wrapper around `stellar --version`; the decision
/// itself lives in [`check_stellar_cli`], which is unit-tested.  Not
/// unit-tested here because it needs a real `stellar` on `PATH`.
fn detect_stellar_cli_version() -> Option<String> {
    let output = std::process::Command::new("stellar")
        .arg("--version")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().next().map(|l| l.trim().to_string())
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

/// Whether an optimize run actually shrank the wasm.
///
/// Derived from [`OptimizeReport::saved_bytes`]: any positive saving is
/// [`OptimizeStatus::Optimized`], while a zero (or, pathologically, negative)
/// saving is [`OptimizeStatus::AlreadyMinimal`].  Automated consumers can
/// branch on this in JSON output instead of treating `saved_bytes == 0` as
/// ambiguous (issue #416).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OptimizeStatus {
    /// The run reduced the wasm size.
    Optimized,
    /// The wasm was already as small as `stellar contract optimize` can make
    /// it — no bytes were saved.
    AlreadyMinimal,
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
    /// Whether any bytes were saved, derived from `saved_bytes` at
    /// construction so it can never disagree with it.
    pub status: OptimizeStatus,
}

impl OptimizeReport {
    pub fn new(wasm: &Path, optimized_wasm: &Path, before_bytes: u64, after_bytes: u64) -> Self {
        let saved_bytes = before_bytes as i64 - after_bytes as i64;
        let status = if saved_bytes > 0 {
            OptimizeStatus::Optimized
        } else {
            OptimizeStatus::AlreadyMinimal
        };
        Self {
            wasm: wasm.display().to_string(),
            optimized_wasm: optimized_wasm.display().to_string(),
            before_bytes,
            after_bytes,
            saved_bytes,
            status,
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

/// Optimize the wasm at `wasm_path` via `stellar contract optimize` and
/// report the size delta.
///
/// With `in_place = false` (the default) the optimized bytes land at
/// `<stem>.optimized.wasm` next to the input and both files remain — the
/// historical behavior.  With `in_place = true` the optimized bytes replace
/// the original file and the intermediate `.optimized.wasm` is removed, so a
/// deploy pipeline can consume a single canonical path.  This also covers the
/// "clean up the pre-optimization wasm" requirement: after a successful
/// in-place run, only the original filename remains on disk.
pub fn optimize(wasm_path: &Path, in_place: bool) -> Result<OptimizeReport> {
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

    if in_place {
        replace_with_optimized(wasm_path, &optimized_path)?;
        // The optimized bytes now live at the original path.
        Ok(OptimizeReport::new(
            wasm_path,
            wasm_path,
            before_bytes,
            after_bytes,
        ))
    } else {
        Ok(OptimizeReport::new(
            wasm_path,
            &optimized_path,
            before_bytes,
            after_bytes,
        ))
    }
}

/// Human-readable report, printed unless `--quiet`.
///
/// Wording distinguishes the two statuses (issue #416): a genuine reduction
/// prints before/after/saved, while an already-minimal run collapses to a
/// single `size` line and never prints a misleading "saved 0 bytes".
pub fn format_report(report: &OptimizeReport) -> String {
    match report.status {
        OptimizeStatus::Optimized => format!(
            "optimized {} -> {}\n\n  before  {} bytes\n  after   {} bytes\n  saved   {} bytes ({:.1}%)\n",
            report.wasm,
            report.optimized_wasm,
            report.before_bytes,
            report.after_bytes,
            report.saved_bytes,
            report.percent_saved(),
        ),
        OptimizeStatus::AlreadyMinimal => format!(
            "already minimal: {}\n\n  size    {} bytes (no reduction)\n",
            report.wasm, report.after_bytes,
        ),
    }
}

/// The same report as JSON, for `--json`.
///
/// `status` is serialized by the `Serialize` derive as one of
/// `"optimized"` / `"already-minimal"`.  `percent_saved` is injected here
/// because it is a computed convenience, not a stored field.
pub fn json_report(report: &OptimizeReport) -> String {
    let mut value = match serde_json::to_value(report) {
        Ok(value) => value,
        Err(e) => return format!("{{\"error\":\"{e}\"}}"),
    };
    if let Some(object) = value.as_object_mut() {
        object.insert("percent_saved".into(), serde_json::json!(report.percent_saved()));
    }
    serde_json::to_string_pretty(&value).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

/// Select the report output, or suppress it entirely in quiet mode.
pub fn report_output(report: &OptimizeReport, json: bool, quiet: bool) -> Option<String> {
    if quiet {
        None
    } else if json {
        Some(json_report(report))
    } else {
        Some(format_report(report))
    }
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
                Arg::new("in-place")
                    .long("in-place")
                    .help("Overwrite the input wasm with the optimized bytes and remove the .optimized.wasm file (leaves only the original filename, holding the optimized bytes)")
                    .action(clap::ArgAction::SetTrue),
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
        // Pre-flight the installed `stellar-cli` before touching the local
        // build or shelling out (issue #415).  `stellar --version` is cheap
        // and offline-safe, so it always runs.
        check_stellar_cli(detect_stellar_cli_version().as_deref())?;

        let dir = matches
            .get_one::<String>("path")
            .map(|p| ctx.cwd.join(p))
            .unwrap_or_else(|| ctx.cwd.clone());
        let wasm_override = matches.get_one::<String>("wasm").map(|p| ctx.cwd.join(p));

        let wasm_path = resolve_local_wasm(&dir, wasm_override.as_deref())?;

        let in_place = matches.get_flag("in-place");

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

        let report = optimize(&wasm_path, in_place)?;
        check_budget(&report, max_size)?;

        if let Some(output) = report_output(&report, ctx.json, ctx.quiet) {
            print!("{output}");
            if !output.ends_with('\n') {
                println!();
            }
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

    /// `replace_with_optimized` overwrites the original file with the
    /// optimized bytes and removes the intermediate `.optimized.wasm`,
    /// leaving only the original filename on disk.
    #[test]
    fn in_place_replaces_original_and_removes_intermediate() {
        let tmp = tempfile::tempdir().unwrap();
        let original = tmp.path().join("token.wasm");
        let optimized = optimized_wasm_path(&original);

        std::fs::write(&original, b"original-bytes").unwrap();
        std::fs::write(&optimized, b"optimized-bytes").unwrap();

        replace_with_optimized(&original, &optimized).unwrap();

        // Only the original filename remains — no `.optimized.wasm`, no `.tmp`.
        let entries: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from("token.wasm")]);
        assert!(!optimized.exists(), "intermediate should be removed");
        assert!(original.is_file(), "original must remain");

        // And it now holds the optimized bytes.
        assert_eq!(std::fs::read(&original).unwrap(), b"optimized-bytes");
    }

    /// The "preserved on failure" guarantee: if the optimized source cannot
    /// be read, the original file is left untouched and an error is returned.
    /// This is deterministic on every platform and privilege level.
    #[test]
    fn replace_with_optimized_preserves_original_when_optimized_is_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let original = tmp.path().join("token.wasm");
        let missing = optimized_wasm_path(&original);

        std::fs::write(&original, b"original-bytes").unwrap();
        // Note: `missing` is deliberately not created.

        let err = replace_with_optimized(&original, &missing).unwrap_err();
        assert!(
            err.to_string().contains("token.optimized.wasm"),
            "error should name the missing source: {err}"
        );

        assert_eq!(
            std::fs::read(&original).unwrap(),
            b"original-bytes",
            "original must be byte-for-byte untouched"
        );
        assert!(!missing.exists());
    }

    /// If the destination directory cannot be written to, the staging write
    /// fails and the original is preserved.  On filesystems / processes where
    /// `chmod` has no effect (e.g. running as root in a container), the write
    /// will succeed and we skip the assertion rather than fail spuriously.
    #[cfg(unix)]
    #[test]
    fn replace_with_optimized_preserves_original_when_destination_is_readonly() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let original = tmp.path().join("token.wasm");
        let optimized = optimized_wasm_path(&original);

        std::fs::write(&original, b"original-bytes").unwrap();
        std::fs::write(&optimized, b"optimized-bytes").unwrap();

        // Drop write permission on the containing directory.
        let dir_mode = std::fs::metadata(tmp.path()).unwrap().permissions().mode();
        let mut readonly = std::fs::metadata(tmp.path()).unwrap().permissions();
        readonly.set_mode(dir_mode & !0o222);
        std::fs::set_permissions(tmp.path(), readonly).unwrap();

        let result = replace_with_optimized(&original, &optimized);

        // Restore permissions so the tempdir can be cleaned up.
        let mut restore = std::fs::metadata(tmp.path()).unwrap().permissions();
        restore.set_mode(dir_mode);
        std::fs::set_permissions(tmp.path(), restore).unwrap();

        match result {
            Ok(()) => {
                // Running as root (or on a filesystem that ignores the mode
                // bit).  Nothing to assert — the platform does not let us
                // provoke a write failure.  Not a failure of this test.
            }
            Err(_) => {
                assert_eq!(
                    std::fs::read(&original).unwrap(),
                    b"original-bytes",
                    "original must be untouched when the staging write fails"
                );
            }
        }
    }

    /// Neither success nor failure paths may leave a stale staging file
    /// behind; the naming convention is `<stem>.<pid>.tmp`.
    #[test]
    fn replace_with_optimized_leaves_no_temp_file_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let original = tmp.path().join("token.wasm");
        let optimized = optimized_wasm_path(&original);

        std::fs::write(&original, b"original-bytes").unwrap();
        std::fs::write(&optimized, b"optimized-bytes").unwrap();

        replace_with_optimized(&original, &optimized).unwrap();

        for entry in std::fs::read_dir(tmp.path()).unwrap() {
            let name = entry.unwrap().file_name();
            let s = name.to_string_lossy();
            assert!(
                !s.ends_with(".tmp"),
                "staging file leaked after success: {s}"
            );
        }

        // Now provoke a failure and assert no `.tmp` leaks either.
        let missing = tmp.path().join("nope.optimized.wasm");
        let _ = replace_with_optimized(&original, &missing);

        for entry in std::fs::read_dir(tmp.path()).unwrap() {
            let name = entry.unwrap().file_name();
            let s = name.to_string_lossy();
            assert!(
                !s.ends_with(".tmp"),
                "staging file leaked after failure: {s}"
            );
        }
    }

    /// Non-in-place replacement path is not exercised by `optimize` without
    /// `stellar`, so this pins the file-system guarantee the default branch
    /// relies on: the intermediate is produced and left alone.
    #[test]
    fn default_mode_leaves_both_files_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let original = tmp.path().join("token.wasm");
        let optimized = optimized_wasm_path(&original);

        std::fs::write(&original, b"original-bytes").unwrap();
        std::fs::write(&optimized, b"optimized-bytes").unwrap();

        // No call to `replace_with_optimized` — mirror the default branch.
        assert_eq!(std::fs::read(&original).unwrap(), b"original-bytes");
        assert_eq!(std::fs::read(&optimized).unwrap(), b"optimized-bytes");
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
        assert_eq!(report.status, OptimizeStatus::Optimized);
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
        assert_eq!(report.status, OptimizeStatus::AlreadyMinimal);
    }

    /// The status field is derived from `saved_bytes`, so zero *or* negative
    /// savings both map to `AlreadyMinimal` (issue #416).
    #[test]
    fn status_is_already_minimal_when_no_bytes_saved() {
        let zero = OptimizeReport::new(
            Path::new("a.wasm"),
            Path::new("a.optimized.wasm"),
            1000,
            1000,
        );
        assert_eq!(zero.status, OptimizeStatus::AlreadyMinimal);

        // Pathological: the optimizer made the wasm *larger*.
        let grew = OptimizeReport::new(
            Path::new("a.wasm"),
            Path::new("a.optimized.wasm"),
            1000,
            1001,
        );
        assert_eq!(grew.saved_bytes, -1);
        assert_eq!(grew.status, OptimizeStatus::AlreadyMinimal);
    }

    #[test]
    fn format_report_includes_sizes() {
        let report = OptimizeReport::new(
            Path::new("a.wasm"),
            Path::new("a.optimized.wasm"),
            1000,
            750,
        );
        assert_eq!(report.status, OptimizeStatus::Optimized);
        let text = format_report(&report);
        assert!(text.contains("optimized"), "{text}");
        assert!(text.contains("1000 bytes"), "{text}");
        assert!(text.contains("750 bytes"), "{text}");
        assert!(text.contains("250 bytes"), "{text}");
        assert!(text.contains("25.0%"), "{text}");
        assert!(!text.contains("already minimal"), "{text}");
    }

    /// Human output for an already-minimal run reads differently from a
    /// real reduction, and never prints "saved 0 bytes" (issue #416).
    #[test]
    fn format_report_says_already_minimal() {
        let report = OptimizeReport::new(
            Path::new("a.wasm"),
            Path::new("a.optimized.wasm"),
            1000,
            1000,
        );
        let text = format_report(&report);
        assert!(text.contains("already minimal"), "{text}");
        assert!(text.contains("1000 bytes"), "{text}");
        assert!(text.contains("no reduction"), "{text}");
        assert!(!text.contains("saved"), "{text}");
        assert!(!text.contains("0.0%"), "{text}");
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
        assert_eq!(parsed["percent_saved"], 25.0);
        assert_eq!(parsed["status"], "optimized");
    }

    /// The JSON status distinguishes a real reduction from an
    /// already-minimal run (issue #416).
    #[test]
    fn json_report_carries_status_for_both_values() {
        let optimized = OptimizeReport::new(
            Path::new("a.wasm"),
            Path::new("a.optimized.wasm"),
            1000,
            750,
        );
        let parsed: serde_json::Value =
            serde_json::from_str(&json_report(&optimized)).unwrap();
        assert_eq!(parsed["status"], "optimized");

        let minimal = OptimizeReport::new(
            Path::new("a.wasm"),
            Path::new("a.optimized.wasm"),
            1000,
            1000,
        );
        let parsed: serde_json::Value = serde_json::from_str(&json_report(&minimal)).unwrap();
        assert_eq!(parsed["status"], "already-minimal");
        assert_eq!(parsed["saved_bytes"], 0);
    }

    #[test]
    fn quiet_suppresses_text_and_json_reports() {
        let report = OptimizeReport::new(
            Path::new("a.wasm"),
            Path::new("a.optimized.wasm"),
            1000,
            750,
        );
        assert_eq!(report_output(&report, false, true), None);
        assert_eq!(report_output(&report, true, true), None);
        assert!(report_output(&report, false, false).unwrap().contains("25.0%"));
        assert!(report_output(&report, true, false).unwrap().contains("percent_saved"));
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

    #[test]
    fn help_documents_in_place_flag() {
        let help = OptimizePlugin.command().render_long_help().to_string();
        assert!(help.contains("--in-place"), "{help}");
    }

    // ---- pre-flight version gate (issue #415) ----

    #[test]
    fn version_gate_rejects_missing_binary() {
        let err = check_stellar_cli(None).unwrap_err();
        assert_eq!(err.exit_code(), soroban_forge_core::error::ExitCode::ToolMissing);
        let rendered = err.to_string();
        assert!(rendered.contains("stellar-cli"), "{rendered}");
        assert!(rendered.contains("soroban-forge doctor"), "{rendered}");
    }

    #[test]
    fn version_gate_rejects_too_old_version() {
        let err = check_stellar_cli(Some("stellar-cli 20.3.0")).unwrap_err();
        assert_eq!(err.exit_code(), soroban_forge_core::error::ExitCode::ToolMissing);
        let rendered = err.to_string();
        assert!(rendered.contains("20.3.0"), "{rendered}");
        assert!(rendered.contains("too old"), "{rendered}");
        assert!(rendered.contains("21.0"), "{rendered}");
        assert!(rendered.contains("soroban-forge doctor"), "{rendered}");
    }

    #[test]
    fn version_gate_rejects_known_broken_version_with_recommendation() {
        for (broken, recommended) in [
            ("stellar-cli 27.0.0", "27.0.1"),
            ("stellar-cli 27.0.1", "27.1.0"),
            ("stellar-cli 28.0.0", "28.0.1"),
            ("stellar-cli 29.0.0", "29.0.1"),
        ] {
            let err = check_stellar_cli(Some(broken)).unwrap_err();
            assert_eq!(
                err.exit_code(),
                soroban_forge_core::error::ExitCode::ToolMissing,
                "{broken}"
            );
            let rendered = err.to_string();
            assert!(rendered.contains("known-broken"), "{broken}: {rendered}");
            assert!(rendered.contains(recommended), "{broken}: {rendered}");
            assert!(rendered.contains("soroban-forge doctor"), "{broken}: {rendered}");
        }
    }

    #[test]
    fn version_gate_accepts_supported_versions() {
        assert!(check_stellar_cli(Some("stellar-cli 21.0.0")).is_ok());
        assert!(check_stellar_cli(Some("stellar-cli 26.1.0")).is_ok());
        assert!(check_stellar_cli(Some("stellar-cli 30.0.0")).is_ok());
        // Trailing details after the version are fine (real --version output).
        assert!(check_stellar_cli(Some("stellar 21.0.0 (abc 2025-01-01)")).is_ok());
    }

    #[test]
    fn version_gate_rejects_unparseable_version_line() {
        // Unparseable versions count as "too old" via version_at_least.
        let err = check_stellar_cli(Some("garbage")).unwrap_err();
        let rendered = err.to_string();
        assert!(rendered.contains("too old"), "{rendered}");
    }
}
