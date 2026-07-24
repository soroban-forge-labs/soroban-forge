//! # soroban-forge-doctor
//!
//! `soroban-forge doctor` — checks that the local environment can build and
//! deploy Soroban contracts, and prints fix instructions for anything
//! missing:
//!
//! - Rust toolchain (`rustc`, `cargo`) at the minimum supported version
//! - the `wasm32v1-none` compilation target
//! - the official `stellar` CLI
//! - `git` (recommended, not required)
//! - when run inside a contract project: the project's `soroban-sdk`
//!   version, compared against the version pinned into new projects
//!   (`soroban_forge_scaffold::SOROBAN_SDK_VERSION`)

use std::path::Path;

use clap::{ArgMatches, Command};
use soroban_forge_core::{ForgeContext, ForgeError, ForgePlugin, Result};
use soroban_forge_scaffold::SOROBAN_SDK_VERSION;

/// Minimum Rust version able to target `wasm32v1-none`.
pub const MIN_RUST: (u32, u32) = (1, 84); // minimum major.minor

/// Outcome of a single environment check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Requirement met.
    Pass,
    /// Missing but not required for local development.
    Warn,
    /// Required and missing/broken.
    Fail,
}

/// One line of the doctor report.
#[derive(Debug, serde::Serialize)]
pub struct Check {
    pub name: &'static str,
    pub status: Status,
    /// What was found, e.g. the tool's version line.
    pub detail: String,
    /// How to fix it, shown for non-passing checks.
    pub fix: Option<&'static str>,
}

/// Run `cmd args...` and return its first line of stdout on success.
fn capture(cmd: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(cmd).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().next().map(|l| l.trim().to_string())
}

/// Parse `X.Y` out of a `tool X.Y.Z ...` version line and compare against a
/// minimum. Unparseable versions count as too old.
pub fn version_at_least(version_line: &str, min: (u32, u32)) -> bool {
    version_line
        .split_whitespace()
        .find_map(|word| {
            let mut parts = word.split('.');
            let major: u32 = parts.next()?.parse().ok()?;
            let minor: u32 = parts
                .next()?
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .ok()?;
            Some((major, minor))
        })
        .map(|version| version >= min)
        .unwrap_or(false)
}

/// Leniently parse a cargo version requirement (e.g. `26.1.0`, `^26.1`,
/// `=26.1.0`, `>=26, <27`) into `(major, minor, patch)`. Missing components
/// default to zero. Returns `None` for wildcards or anything else that does
/// not start with a numeric major version.
pub fn parse_semverish(version: &str) -> Option<(u32, u32, u32)> {
    let first = version
        .split(',')
        .next()?
        .trim()
        .trim_start_matches(['^', '~', '=', '>', '<', 'v', ' ']);
    let mut parts = first.split('.');
    let major: u32 = parts.next()?.trim().parse().ok()?;
    let minor: u32 = parts
        .next()
        .unwrap_or("0")
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or(0);
    let patch: u32 = parts
        .next()
        .unwrap_or("0")
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or(0);
    Some((major, minor, patch))
}

/// Extract the declared `soroban-sdk` version from a parsed manifest.
///
/// Outer `Option`: whether the manifest declares a `soroban-sdk` dependency
/// at all. Inner `Option`: the version requirement, `None` when the
/// dependency has no `version` key (e.g. a git or path dependency).
fn manifest_sdk_version(manifest: &toml::Value) -> Option<Option<String>> {
    for table in ["dependencies", "dev-dependencies"] {
        if let Some(dep) = manifest.get(table).and_then(|t| t.get("soroban-sdk")) {
            return Some(dep_version(dep));
        }
    }
    manifest
        .get("workspace")
        .and_then(|w| w.get("dependencies"))
        .and_then(|t| t.get("soroban-sdk"))
        .map(dep_version)
}

/// The version requirement of a single dependency entry, covering both the
/// string form (`soroban-sdk = "26.1.0"`) and the table form
/// (`soroban-sdk = { version = "26.1.0", ... }`).
fn dep_version(dep: &toml::Value) -> Option<String> {
    match dep {
        toml::Value::String(s) => Some(s.clone()),
        other => other
            .get("version")
            .and_then(|v| v.as_str())
            .map(String::from),
    }
}

/// Check the project's `soroban-sdk` version against the version pinned into
/// freshly scaffolded projects.
///
/// Returns `None` (no report line at all) when `project_dir` does not look
/// like a contract project: no readable/parseable `Cargo.toml`, or a
/// manifest without a `soroban-sdk` dependency. Otherwise:
///
/// - `Pass` when the declared version is at or above the pinned one
/// - `Warn` when it is behind, unversioned, or unparseable
pub fn sdk_version_check(project_dir: &Path) -> Option<Check> {
    let contents = std::fs::read_to_string(project_dir.join("Cargo.toml")).ok()?;
    let manifest: toml::Value = toml::from_str(&contents).ok()?;
    let declared = manifest_sdk_version(&manifest)?;
    let pinned = parse_semverish(SOROBAN_SDK_VERSION)?;

    Some(match declared {
        None => Check {
            name: "soroban-sdk",
            status: Status::Warn,
            detail: format!("no version specified (latest pinned: {SOROBAN_SDK_VERSION})"),
            fix: Some("pin a soroban-sdk version in Cargo.toml"),
        },
        Some(raw) => match parse_semverish(&raw) {
            Some(found) if found >= pinned => Check {
                name: "soroban-sdk",
                status: Status::Pass,
                detail: format!("soroban-sdk {raw}"),
                fix: None,
            },
            Some(_) => Check {
                name: "soroban-sdk",
                status: Status::Warn,
                detail: format!("soroban-sdk {raw} (latest pinned: {SOROBAN_SDK_VERSION})"),
                fix: Some("update the soroban-sdk version in Cargo.toml"),
            },
            None => Check {
                name: "soroban-sdk",
                status: Status::Warn,
                detail: format!(
                    "could not parse version `{raw}` (latest pinned: {SOROBAN_SDK_VERSION})"
                ),
                fix: Some("pin a concrete soroban-sdk version in Cargo.toml"),
            },
        },
    })
}

/// Run all environment checks.
pub fn run_checks() -> Vec<Check> {
    let mut checks = Vec::new();

    // rustc, with a minimum version.
    checks.push(match capture("rustc", &["--version"]) {
        Some(line) if version_at_least(&line, MIN_RUST) => Check {
            name: "rustc",
            status: Status::Pass,
            detail: line,
            fix: None,
        },
        Some(line) => Check {
            name: "rustc",
            status: Status::Fail,
            detail: format!("{line} (need >= {}.{})", MIN_RUST.0, MIN_RUST.1),
            fix: Some("update Rust: rustup update stable"),
        },
        None => Check {
            name: "rustc",
            status: Status::Fail,
            detail: "not found".into(),
            fix: Some("install Rust: https://rustup.rs"),
        },
    });

    // cargo.
    checks.push(match capture("cargo", &["--version"]) {
        Some(line) => Check {
            name: "cargo",
            status: Status::Pass,
            detail: line,
            fix: None,
        },
        None => Check {
            name: "cargo",
            status: Status::Fail,
            detail: "not found".into(),
            fix: Some("install Rust (includes cargo): https://rustup.rs"),
        },
    });

    // wasm32v1-none target.
    let installed_targets = std::process::Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    checks.push(match installed_targets {
        Some(targets) if targets.lines().any(|t| t.trim() == "wasm32v1-none") => Check {
            name: "wasm32v1-none target",
            status: Status::Pass,
            detail: "installed".into(),
            fix: None,
        },
        Some(_) => Check {
            name: "wasm32v1-none target",
            status: Status::Fail,
            detail: "not installed".into(),
            fix: Some("rustup target add wasm32v1-none"),
        },
        None => Check {
            name: "wasm32v1-none target",
            status: Status::Warn,
            detail: "rustup not found — could not verify".into(),
            fix: Some("install rustup (https://rustup.rs), then: rustup target add wasm32v1-none"),
        },
    });

    // stellar-cli.
    checks.push(match capture("stellar", &["--version"]) {
        Some(line) => Check {
            name: "stellar-cli",
            status: Status::Pass,
            detail: line,
            fix: None,
        },
        None => Check {
            name: "stellar-cli",
            status: Status::Fail,
            detail: "not found".into(),
            fix: Some(
                "install: brew install stellar-cli  (or: cargo install --locked stellar-cli)",
            ),
        },
    });

    // git — recommended only.
    checks.push(match capture("git", &["--version"]) {
        Some(line) => Check {
            name: "git",
            status: Status::Pass,
            detail: line,
            fix: None,
        },
        None => Check {
            name: "git",
            status: Status::Warn,
            detail: "not found".into(),
            fix: Some("install git: https://git-scm.com/downloads"),
        },
    });

    checks
}

/// Render the report as shown to the user.
pub fn format_report(checks: &[Check]) -> String {
    let mut out = String::from("soroban-forge doctor\n\n");
    for check in checks {
        let symbol = match check.status {
            Status::Pass => "✓",
            Status::Warn => "!",
            Status::Fail => "✗",
        };
        out.push_str(&format!("  {symbol} {:<22} {}\n", check.name, check.detail));
        if check.status != Status::Pass {
            if let Some(fix) = check.fix {
                out.push_str(&format!("      fix: {fix}\n"));
            }
        }
    }
    let failures = checks.iter().filter(|c| c.status == Status::Fail).count();
    let warnings = checks.iter().filter(|c| c.status == Status::Warn).count();
    out.push('\n');
    if failures == 0 && warnings == 0 {
        out.push_str("all checks passed — you're ready to build Soroban contracts.\n");
    } else {
        out.push_str(&format!("{failures} failure(s), {warnings} warning(s).\n"));
    }
    out
}

/// Count required checks that failed.
pub fn failure_count(checks: &[Check]) -> usize {
    checks
        .iter()
        .filter(|check| check.status == Status::Fail)
        .count()
}

/// The `doctor` subcommand.
pub struct DoctorPlugin;

impl ForgePlugin for DoctorPlugin {
    fn name(&self) -> &'static str {
        "doctor"
    }

    fn command(&self) -> Command {
        Command::new("doctor").about(
            "Check that Rust, the wasm32v1-none target and stellar-cli are installed, \
             and that the project's soroban-sdk is up to date",
        )
    }

    fn run(&self, _matches: &ArgMatches, ctx: &ForgeContext) -> Result<()> {
        let mut checks = run_checks();
        // Project-local check: only reports when run inside a contract project.
        if let Some(check) = sdk_version_check(&ctx.cwd) {
            checks.push(check);
        }
        if ctx.json {
            let failures = checks.iter().filter(|c| c.status == Status::Fail).count();
            let warnings = checks.iter().filter(|c| c.status == Status::Warn).count();
            let report = serde_json::json!({
                "checks": checks,
                "failures": failures,
                "warnings": warnings,
            });
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
        } else if !ctx.quiet {
            print!("{}", format_report(&checks));
        }
        let failures = failure_count(&checks);
        if failures > 0 {
            Err(ForgeError::Doctor(format!(
                "{failures} required check(s) failed"
            )))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison() {
        assert!(version_at_least("rustc 1.84.0 (abc 2025-01-01)", (1, 84)));
        assert!(version_at_least("rustc 1.90.1-nightly", (1, 84)));
        assert!(version_at_least("cargo 2.0.0", (1, 84)));
        assert!(!version_at_least("rustc 1.83.0", (1, 84)));
        assert!(!version_at_least("garbage", (1, 84)));
    }

    #[test]
    fn report_lists_fixes_for_failures() {
        let checks = vec![
            Check {
                name: "rustc",
                status: Status::Pass,
                detail: "rustc 1.90.0".into(),
                fix: None,
            },
            Check {
                name: "stellar-cli",
                status: Status::Fail,
                detail: "not found".into(),
                fix: Some("install: brew install stellar-cli"),
            },
        ];
        let report = format_report(&checks);
        assert!(report.contains("✓ rustc"));
        assert!(report.contains("✗ stellar-cli"));
        assert!(report.contains("fix: install: brew install stellar-cli"));
        assert!(report.contains("1 failure(s)"));
    }

    #[test]
    fn all_pass_report() {
        let checks = vec![Check {
            name: "rustc",
            status: Status::Pass,
            detail: "ok".into(),
            fix: None,
        }];
        assert!(format_report(&checks).contains("all checks passed"));
    }

    #[test]
    fn failure_count_ignores_passes_and_warnings() {
        let checks = [
            Check {
                name: "pass",
                status: Status::Pass,
                detail: String::new(),
                fix: None,
            },
            Check {
                name: "warn",
                status: Status::Warn,
                detail: String::new(),
                fix: None,
            },
            Check {
                name: "fail",
                status: Status::Fail,
                detail: String::new(),
                fix: None,
            },
        ];
        assert_eq!(failure_count(&checks), 1);
    }

    #[test]
    fn successful_checks_have_zero_failures() {
        let checks = [Check {
            name: "rustc",
            status: Status::Pass,
            detail: "rustc 1.84.0".into(),
            fix: None,
        }];
        assert_eq!(failure_count(&checks), 0);
    }

    // ---- soroban-sdk version check ----

    fn project_with_manifest(manifest: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), manifest).unwrap();
        dir
    }

    #[test]
    fn parses_common_version_requirements() {
        assert_eq!(parse_semverish("26.1.0"), Some((26, 1, 0)));
        assert_eq!(parse_semverish("^26.1"), Some((26, 1, 0)));
        assert_eq!(parse_semverish("=25.0.3"), Some((25, 0, 3)));
        assert_eq!(parse_semverish(">=26, <27"), Some((26, 0, 0)));
        assert_eq!(parse_semverish("26"), Some((26, 0, 0)));
        assert_eq!(parse_semverish("1.2.3-rc.1"), Some((1, 2, 3)));
        assert_eq!(parse_semverish("*"), None);
        assert_eq!(parse_semverish("garbage"), None);
    }

    #[test]
    fn pinned_sdk_version_is_parseable() {
        assert!(
            parse_semverish(SOROBAN_SDK_VERSION).is_some(),
            "scaffold::SOROBAN_SDK_VERSION must be a parseable semver"
        );
    }

    #[test]
    fn no_op_without_manifest() {
        let dir = tempfile::tempdir().unwrap();
        assert!(sdk_version_check(dir.path()).is_none());
    }

    #[test]
    fn no_op_without_soroban_sdk_dependency() {
        let dir = project_with_manifest(
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n\n[dependencies]\nserde = \"1\"\n",
        );
        assert!(sdk_version_check(dir.path()).is_none());
    }

    #[test]
    fn warns_when_project_sdk_is_behind() {
        let dir = project_with_manifest(
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n\n[dependencies]\nsoroban-sdk = \"25.0.0\"\n",
        );
        let check = sdk_version_check(dir.path()).unwrap();
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("25.0.0"));
        assert!(check.detail.contains(SOROBAN_SDK_VERSION));
        assert!(check.fix.is_some());
    }

    #[test]
    fn passes_when_project_sdk_is_current() {
        let dir = project_with_manifest(&format!(
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n\n[dependencies]\nsoroban-sdk = \"{SOROBAN_SDK_VERSION}\"\n",
        ));
        let check = sdk_version_check(dir.path()).unwrap();
        assert_eq!(check.status, Status::Pass);
        assert!(check.fix.is_none());
    }

    #[test]
    fn passes_when_project_sdk_is_ahead() {
        let dir = project_with_manifest(
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n\n[dependencies]\nsoroban-sdk = \"99.0.0\"\n",
        );
        assert_eq!(sdk_version_check(dir.path()).unwrap().status, Status::Pass);
    }

    #[test]
    fn reads_table_form_and_dev_dependencies() {
        let table = project_with_manifest(
            "[dependencies]\nsoroban-sdk = { version = \"25.1.2\", features = [\"testutils\"] }\n",
        );
        let check = sdk_version_check(table.path()).unwrap();
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("25.1.2"));

        let dev = project_with_manifest(&format!(
            "[dev-dependencies]\nsoroban-sdk = \"{SOROBAN_SDK_VERSION}\"\n"
        ));
        assert_eq!(sdk_version_check(dev.path()).unwrap().status, Status::Pass);
    }

    #[test]
    fn warns_on_versionless_dependency() {
        let dir = project_with_manifest(
            "[dependencies]\nsoroban-sdk = { git = \"https://example.com/sdk\" }\n",
        );
        let check = sdk_version_check(dir.path()).unwrap();
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("no version specified"));
    }
}
