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

use clap::{ArgMatches, Command};
use soroban_forge_core::{ForgeContext, ForgeError, ForgePlugin, Result};

/// Minimum Rust version able to target `wasm32v1-none`.
pub const MIN_RUST: (u32, u32) = (1, 84);

/// Outcome of a single environment check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Requirement met.
    Pass,
    /// Missing but not required for local development.
    Warn,
    /// Required and missing/broken.
    Fail,
}

/// One line of the doctor report.
#[derive(Debug)]
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

/// The `doctor` subcommand.
pub struct DoctorPlugin;

impl ForgePlugin for DoctorPlugin {
    fn name(&self) -> &'static str {
        "doctor"
    }

    fn command(&self) -> Command {
        Command::new("doctor")
            .about("Check that Rust, the wasm32v1-none target and stellar-cli are installed")
    }

    fn run(&self, _matches: &ArgMatches, _ctx: &ForgeContext) -> Result<()> {
        let checks = run_checks();
        print!("{}", format_report(&checks));
        let failures = checks.iter().filter(|c| c.status == Status::Fail).count();
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
}
