//! # soroban-forge-doctor
//!
//! `soroban-forge doctor` — checks that the local environment can build and
//! deploy Soroban contracts, and prints fix instructions for anything
//! missing:
//!
//! - Rust toolchain (`rustc`, `cargo`) at the minimum supported version
//! - the `wasm32v1-none` compilation target
//! - the official `stellar` CLI
//! - `git` (recommended, not required), and its `user.name`/`user.email`
//!   identity, without which the first commit in a new project fails
//! - Docker (optional, used for reproducible wasm builds)
//! - free disk space (optional warning when below 1 GiB, since wasm/target builds can be large)
//! - when run inside a contract project: the project's `soroban-sdk`
//!   version, compared against the latest stable version published on
//!   crates.io (falling back to the version pinned into new projects when
//!   crates.io cannot be reached)
//! - when run inside a cargo project: a `Cargo.lock` that version control
//!   will carry, since a missing or gitignored lockfile leaves CI resolving
//!   fresh dependency versions on every build
//!
//! With `--fix`, doctor will, after confirmation, run the subset of remedies
//! that are safe to automate (`rustup target add`, `cargo install`), then
//! re-check and report anything still outstanding. `--yes`/`-y` skips the
//! prompt for non-interactive use.

use std::path::Path;

use clap::{Arg, ArgAction, ArgMatches, Command};
use serde::{Deserialize, Serialize};
use soroban_forge_core::toolchain::{known_broken_stellar_cli_replacement, MIN_RUST, MIN_STELLAR};
use soroban_forge_core::{ForgeContext, ForgeError, ForgePlugin, Result};
use soroban_forge_scaffold::SOROBAN_SDK_VERSION;

// Re-exported so external callers of `soroban_forge_doctor::version_at_least`
// and `soroban_forge_doctor::parse_semverish` keep working after these
// helpers moved into `soroban-forge-core` (issue #415).
pub use soroban_forge_core::toolchain::{parse_semverish, version_at_least};

/// Default Soroban RPC endpoint used for the connectivity check.
pub const TESTNET_RPC_URL: &str = "https://soroban-testnet.stellar.org";

/// Minimum recommended free disk space (in bytes) for building Soroban contracts and target artifacts (1 GiB).
pub const MIN_FREE_DISK_SPACE_BYTES: u64 = 1024 * 1024 * 1024; // 1 GiB

/// Outcome of a single environment check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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

/// Like [`capture`], but runs the command with `dir` as its working
/// directory. Needed for checks whose answer depends on the project — e.g.
/// `rustup show active-toolchain` honours a local `rust-toolchain.toml`
/// override or `RUSTUP_TOOLCHAIN` only when run from inside the project.
fn capture_in(cmd: &str, args: &[&str], dir: &Path) -> Option<String> {
    let output = std::process::Command::new(cmd)
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().next().map(|l| l.trim().to_string())
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

/// Fetch the latest stable `soroban-sdk` version published on crates.io.
///
/// This check is advisory, so registry/network errors are intentionally
/// ignored by the caller and fall back to the version pinned in templates.
fn latest_published_sdk_version() -> Option<String> {
    let response = ureq::get("https://crates.io/api/v1/crates/soroban-sdk")
        .set("User-Agent", "soroban-forge")
        .timeout(std::time::Duration::from_secs(3))
        .call()
        .ok()?
        .into_string()
        .ok()?;
    let body: serde_json::Value = serde_json::from_str(&response).ok()?;
    let krate = body.get("crate")?;
    let version = krate
        .get("max_stable_version")
        .and_then(serde_json::Value::as_str)
        .or_else(|| krate.get("max_version").and_then(serde_json::Value::as_str))?;
    parse_semverish(version).map(|_| version.to_owned())
}

/// Check the project's `soroban-sdk` version against the latest published
/// stable version (or the template pin when the registry is unavailable).
///
/// Returns `None` (no report line at all) when `project_dir` does not look
/// like a contract project: no readable/parseable `Cargo.toml`, or a
/// manifest without a `soroban-sdk` dependency. Otherwise:
///
/// - `Pass` when the declared version is at or above the latest version
/// - `Warn` when it is behind, unversioned, or unparseable
pub fn sdk_version_check(project_dir: &Path) -> Option<Check> {
    sdk_version_check_with(project_dir, latest_published_sdk_version)
}

fn sdk_version_check_with(
    project_dir: &Path,
    latest_version: impl FnOnce() -> Option<String>,
) -> Option<Check> {
    let contents = std::fs::read_to_string(project_dir.join("Cargo.toml")).ok()?;
    let manifest: toml::Value = toml::from_str(&contents).ok()?;
    let declared = manifest_sdk_version(&manifest)?;
    let latest = latest_version().filter(|version| parse_semverish(version).is_some());
    let latest_available = latest.is_some();
    let latest = latest.unwrap_or_else(|| SOROBAN_SDK_VERSION.to_owned());
    let latest_parsed = parse_semverish(&latest)?;
    let latest_label = if latest_available {
        format!("latest: {latest}")
    } else {
        format!("latest published unavailable; template pin: {latest}")
    };

    Some(match declared {
        None => Check {
            name: "soroban-sdk",
            status: Status::Warn,
            detail: format!("no version specified ({latest_label})"),
            fix: Some("pin a soroban-sdk version in Cargo.toml"),
        },
        Some(raw) => match parse_semverish(&raw) {
            Some(found) if found >= latest_parsed => Check {
                name: "soroban-sdk",
                status: Status::Pass,
                detail: format!("soroban-sdk {raw} ({latest_label})"),
                fix: None,
            },
            Some(_) => Check {
                name: "soroban-sdk",
                status: Status::Warn,
                detail: format!("soroban-sdk {raw} ({latest_label})"),
                fix: Some("update the soroban-sdk version in Cargo.toml"),
            },
            None => Check {
                name: "soroban-sdk",
                status: Status::Warn,
                detail: format!("could not parse version `{raw}` ({latest_label})"),
                fix: Some("pin a concrete soroban-sdk version in Cargo.toml"),
            },
        },
    })
}

/// Classify a `docker --version` / `docker info` probe into a report line
/// (issue #70).
///
/// Docker is optional: reproducible Soroban wasm builds commonly use it, but
/// local development does not need it, so an absent or stopped daemon is a
/// [`Status::Warn`], never a [`Status::Fail`].
///
/// `version_line` is the first line of `docker --version` (`None` when the
/// binary is missing); `daemon_running` is whether `docker info` succeeded.
pub fn classify_docker(version_line: Option<&str>, daemon_running: bool) -> Check {
    match version_line {
        Some(line) if daemon_running => Check {
            name: "docker",
            status: Status::Pass,
            detail: format!("{line} (daemon running)"),
            fix: None,
        },
        Some(line) => Check {
            name: "docker",
            status: Status::Warn,
            detail: format!("{line} — installed but the daemon is not responding"),
            fix: Some(
                "start Docker (open Docker Desktop, or: sudo systemctl start docker) \
                 — only needed for reproducible wasm builds",
            ),
        },
        None => Check {
            name: "docker",
            status: Status::Warn,
            detail: "not found (optional — used for reproducible wasm builds)".into(),
            fix: Some("install Docker: https://docs.docker.com/get-docker/"),
        },
    }
}

/// Report whether Docker is installed and its daemon is reachable.
///
/// Thin system-touching wrapper around [`classify_docker`].
pub fn docker_check() -> Check {
    let version_line = capture("docker", &["--version"]);
    // `docker info` fails fast when the daemon is not reachable; the format
    // string keeps output to a single short line.
    let daemon_running = version_line.is_some()
        && std::process::Command::new("docker")
            .args(["info", "--format", "{{.ServerVersion}}"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
    classify_docker(version_line.as_deref(), daemon_running)
}

fn rustup_available() -> bool {
    std::process::Command::new("rustup")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn missing_target_fix(target: &str, rustup_present: bool) -> Option<&'static str> {
    match (rustup_present, target) {
        (true, "wasm32v1-none") => Some("rustup target add wasm32v1-none"),
        (true, "wasm32-unknown-unknown") => Some("rustup target add wasm32-unknown-unknown"),
        (false, "wasm32v1-none") => Some(
            "install the wasm32v1-none target via your system package manager or the Rust distribution you installed; rustup is not available on PATH",
        ),
        (false, "wasm32-unknown-unknown") => Some(
            "install the wasm32-unknown-unknown target via your system package manager or the Rust distribution you installed; rustup is not available on PATH",
        ),
        _ => None,
    }
}

fn rust_update_fix(rustup_present: bool) -> &'static str {
    if rustup_present {
        "update Rust: rustup update stable"
    } else {
        "update Rust via your system package manager or the Rust distribution you installed; rustup is not available on PATH"
    }
}

fn rust_install_fix(rustup_present: bool) -> &'static str {
    if rustup_present {
        "install Rust: https://rustup.rs"
    } else {
        "install Rust via your system package manager or the Rust distribution you installed; rustup is not available on PATH"
    }
}

/// Extract the release channel from a toolchain name, e.g.
/// `stable-x86_64-unknown-linux-gnu` -> `stable`. A name that does not start
/// with a known channel word is a version-pinned toolchain (e.g.
/// `1.84.0-x86_64-...`), reported as `"pinned"` rather than guessed at.
fn channel_from_toolchain_name(name: &str) -> &'static str {
    for channel in ["stable", "beta", "nightly"] {
        if name.starts_with(channel) {
            return channel;
        }
    }
    "pinned"
}

/// Classify the resolved active toolchain into a report line (issue #109).
///
/// `rustup_line` is the first line of `rustup show active-toolchain`, run
/// with the project directory as its working directory so a local
/// `rust-toolchain.toml` override is honoured. When `rustup` itself is not
/// on `PATH`, falls back to `rustc_version_line` (`rustc --version`) and
/// infers the channel from the version string instead.
pub fn classify_toolchain(rustup_line: Option<&str>, rustc_version_line: Option<&str>) -> Check {
    if let Some(line) = rustup_line {
        let toolchain = line.split_whitespace().next().unwrap_or(line);
        let channel = channel_from_toolchain_name(toolchain);
        return Check {
            name: "toolchain",
            status: Status::Pass,
            detail: format!("{line} (channel: {channel})"),
            fix: None,
        };
    }
    match rustc_version_line {
        Some(line) => {
            let channel = if line.contains("nightly") {
                "nightly"
            } else if line.contains("beta") {
                "beta"
            } else {
                "stable"
            };
            Check {
                name: "toolchain",
                status: Status::Pass,
                detail: format!("{line} (channel: {channel}, rustup not found)"),
                fix: None,
            }
        }
        None => Check {
            name: "toolchain",
            status: Status::Warn,
            detail: "could not determine active toolchain (rustc not found)".into(),
            fix: Some("install Rust: https://rustup.rs"),
        },
    }
}

/// Report the active Rust toolchain and channel resolved for `project_dir`.
///
/// Thin system-touching wrapper around [`classify_toolchain`].
pub fn toolchain_check(project_dir: &Path) -> Check {
    let rustup_line = capture_in("rustup", &["show", "active-toolchain"], project_dir);
    let rustc_line = capture("rustc", &["--version"]);
    classify_toolchain(rustup_line.as_deref(), rustc_line.as_deref())
}

fn installed_targets_for_toolchain(toolchain: &str) -> Option<Vec<String>> {
    let output = std::process::Command::new("rustup")
        .args(["target", "list", "--installed", "--toolchain", toolchain])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Some(
        stdout
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect(),
    )
}

fn active_toolchain(project_dir: &Path) -> Option<String> {
    capture_in("rustup", &["show", "active-toolchain"], project_dir)
        .or_else(|| capture("rustup", &["show", "active-toolchain"]))
        .and_then(|line| line.split_whitespace().next().map(str::to_owned))
}

fn wasm32_target_check(project_dir: &Path) -> Check {
    let active = active_toolchain(project_dir);
    let active_toolchain_name = active.as_deref().unwrap_or("stable");
    let rustup_present = rustup_available();

    if !rustup_present {
        return Check {
            name: "wasm32v1-none target",
            status: Status::Warn,
            detail: "rustup not found — could not verify whether the target is installed".into(),
            fix: missing_target_fix("wasm32v1-none", false),
        };
    }

    match capture("rustup", &["toolchain", "list"]) {
        Some(toolchains) => {
            let other_toolchains = toolchains
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .filter_map(|line| line.split_whitespace().next())
                .filter(|toolchain| Some(*toolchain) != active.as_deref())
                .collect::<Vec<_>>();

            if let Some(targets) = active.as_deref().and_then(installed_targets_for_toolchain) {
                if targets.iter().any(|t| t == "wasm32v1-none") {
                    return Check {
                        name: "wasm32v1-none target",
                        status: Status::Pass,
                        detail: format!("installed for {active_toolchain_name}"),
                        fix: None,
                    };
                }
            }

            if let Some(other) = other_toolchains.iter().find(|toolchain| {
                installed_targets_for_toolchain(toolchain)
                    .map(|targets| targets.iter().any(|t| t == "wasm32v1-none"))
                    .unwrap_or(false)
            }) {
                let fix =
                    format!("rustup target add --toolchain {active_toolchain_name} wasm32v1-none");
                return Check {
                    name: "wasm32v1-none target",
                    status: Status::Fail,
                    detail: format!(
                        "installed for {other}, active toolchain is {active_toolchain_name}; exact fix: {fix}"
                    ),
                    fix: Some("rustup target add wasm32v1-none"),
                };
            }

            if active.is_some() {
                let fix =
                    format!("rustup target add --toolchain {active_toolchain_name} wasm32v1-none");
                return Check {
                    name: "wasm32v1-none target",
                    status: Status::Fail,
                    detail: format!("not installed for {active_toolchain_name}; exact fix: {fix}"),
                    fix: Some("rustup target add wasm32v1-none"),
                };
            }
        }
        None => {
            return Check {
                name: "wasm32v1-none target",
                status: Status::Warn,
                detail: "rustup not found — could not verify".into(),
                fix: missing_target_fix("wasm32v1-none", false),
            };
        }
    }

    Check {
        name: "wasm32v1-none target",
        status: Status::Fail,
        detail: "not installed".into(),
        fix: missing_target_fix("wasm32v1-none", true),
    }
}

/// Classify a git identity probe into a report line (issue #71).
///
/// `new` initializes a git repo, and the first commit fails confusingly when
/// `user.name`/`user.email` are unset — so a missing identity is a
/// [`Status::Warn`] carrying the exact commands to set it.
pub fn classify_git_identity(name: Option<&str>, email: Option<&str>) -> Check {
    let name = name.map(str::trim).filter(|s| !s.is_empty());
    let email = email.map(str::trim).filter(|s| !s.is_empty());
    match (name, email) {
        (Some(name), Some(email)) => Check {
            name: "git identity",
            status: Status::Pass,
            detail: format!("{name} <{email}>"),
            fix: None,
        },
        (name, email) => {
            let missing = match (name.is_some(), email.is_some()) {
                (false, false) => "user.name and user.email are not set",
                (true, false) => "user.email is not set",
                (false, true) => "user.name is not set",
                (true, true) => unreachable!("both set is handled above"),
            };
            Check {
                name: "git identity",
                status: Status::Warn,
                detail: missing.to_string(),
                fix: Some(
                    "git config --global user.name \"Your Name\"  &&  \
                     git config --global user.email \"you@example.com\"",
                ),
            }
        }
    }
}

/// Report whether git's committer identity is configured.
///
/// Thin system-touching wrapper around [`classify_git_identity`].
pub fn git_identity_check() -> Check {
    let name = capture("git", &["config", "--get", "user.name"]);
    let email = capture("git", &["config", "--get", "user.email"]);
    classify_git_identity(name.as_deref(), email.as_deref())
}

/// Format a byte count into a human-readable string (e.g. `1.2 GB`, `500.0 MB`).
pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * KB;
    const GB: f64 = 1024.0 * MB;
    const TB: f64 = 1024.0 * GB;

    let b = bytes as f64;
    if b >= TB {
        format!("{:.1} TB", b / TB)
    } else if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

/// Query available disk space (in bytes) for the filesystem containing `path`.
#[cfg(windows)]
pub fn available_disk_space(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;

    let mut target_dir = path;
    while !target_dir.exists() {
        if let Some(parent) = target_dir.parent() {
            if parent.as_os_str().is_empty() {
                target_dir = Path::new(".");
                break;
            }
            target_dir = parent;
        } else {
            target_dir = Path::new(".");
            break;
        }
    }

    let mut wide: Vec<u16> = target_dir.as_os_str().encode_wide().collect();
    wide.push(0);

    let mut free_bytes: u64 = 0;
    let mut total_bytes: u64 = 0;
    let mut total_free_bytes: u64 = 0;

    extern "system" {
        fn GetDiskFreeSpaceExW(
            lpDirectoryName: *const u16,
            lpFreeBytesAvailableToCaller: *mut u64,
            lpTotalNumberOfBytes: *mut u64,
            lpTotalNumberOfFreeBytes: *mut u64,
        ) -> i32;
    }

    let ret = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_bytes,
            &mut total_bytes,
            &mut total_free_bytes,
        )
    };

    if ret != 0 {
        Some(free_bytes)
    } else {
        None
    }
}

/// Query available disk space (in bytes) for the filesystem containing `path`.
#[cfg(unix)]
pub fn available_disk_space(path: &Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let mut target_dir = path;
    while !target_dir.exists() {
        if let Some(parent) = target_dir.parent() {
            if parent.as_os_str().is_empty() {
                target_dir = Path::new(".");
                break;
            }
            target_dir = parent;
        } else {
            target_dir = Path::new(".");
            break;
        }
    }

    let c_path = CString::new(target_dir.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let ret = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if ret == 0 {
        let block_size = if stat.f_frsize > 0 {
            stat.f_frsize as u64
        } else {
            stat.f_bsize as u64
        };
        Some(stat.f_bavail as u64 * block_size)
    } else {
        None
    }
}

/// Query available disk space (in bytes) for the filesystem containing `path`.
#[cfg(not(any(windows, unix)))]
pub fn available_disk_space(_path: &Path) -> Option<u64> {
    None
}

/// Classify an available disk space probe into a report line.
///
/// Wasm and target builds can consume significant disk space. Low disk space is a
/// [`Status::Warn`], never a [`Status::Fail`].
pub fn classify_disk_space(available_bytes: Option<u64>, threshold_bytes: u64) -> Check {
    match available_bytes {
        Some(bytes) if bytes >= threshold_bytes => Check {
            name: "disk space",
            status: Status::Pass,
            detail: format!("{} free", format_bytes(bytes)),
            fix: None,
        },
        Some(bytes) => Check {
            name: "disk space",
            status: Status::Warn,
            detail: format!(
                "{} free (low; recommended >= {} for wasm/target builds)",
                format_bytes(bytes),
                format_bytes(threshold_bytes)
            ),
            fix: Some(
                "free up disk space on the active drive (target and wasm builds can be large)",
            ),
        },
        None => Check {
            name: "disk space",
            status: Status::Warn,
            detail: "could not determine available disk space".into(),
            fix: Some("verify filesystem permissions or check disk space manually"),
        },
    }
}

/// Report available disk space for `path` against [`MIN_FREE_DISK_SPACE_BYTES`].
///
/// Thin system-touching wrapper around [`classify_disk_space`].
pub fn disk_space_check(path: &Path) -> Check {
    let available = available_disk_space(path);
    classify_disk_space(available, MIN_FREE_DISK_SPACE_BYTES)
}

/// Check whether `url` is reachable with an HTTP GET, returning latency in ms.
///
/// Uses `curl` as a subprocess to avoid pulling in an HTTP client dependency.
/// A 5-second timeout is applied; failures (network error, timeout, non-2xx)
/// are reported as `Warn` rather than `Fail` so an offline developer is not
/// blocked.
pub fn rpc_connectivity_check(url: &str) -> Check {
    use std::time::Instant;
    let start = Instant::now();
    // -s silent, -o /dev/null discard body, -w write status code,
    // --max-time 5 abort after 5 s, -L follow redirects.
    let output = std::process::Command::new("curl")
        .args([
            "-s",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "--max-time",
            "5",
            "-L",
            url,
        ])
        .output();
    let elapsed_ms = start.elapsed().as_millis();
    match output {
        Ok(o) if o.status.success() => {
            let code_str = String::from_utf8_lossy(&o.stdout);
            let code: u16 = code_str.trim().parse().unwrap_or(0);
            if (200..400).contains(&code) {
                Check {
                    name: "testnet RPC",
                    status: Status::Pass,
                    detail: format!("{url} — HTTP {code} ({elapsed_ms} ms)"),
                    fix: None,
                }
            } else {
                Check {
                    name: "testnet RPC",
                    status: Status::Warn,
                    detail: format!("{url} — HTTP {code} ({elapsed_ms} ms)"),
                    fix: Some(
                        "check your network connection or configure a different RPC endpoint",
                    ),
                }
            }
        }
        Ok(_) | Err(_) => Check {
            name: "testnet RPC",
            status: Status::Warn,
            detail: format!("{url} — unreachable (timeout or network error, {elapsed_ms} ms)"),
            fix: Some("check your network connection or configure a different RPC endpoint"),
        },
    }
}

/// Check that [profile.release] in `Cargo.toml` is size-optimised.
///
/// Looks for `opt-level = "z"`, `lto = true`, and `codegen-units = 1`.
/// Returns one [`Check`] per missing setting (or an empty `Vec` when the
/// manifest is not a Cargo project or already has all three settings).
pub fn release_profile_checks(project_dir: &Path) -> Vec<Check> {
    let contents = match std::fs::read_to_string(project_dir.join("Cargo.toml")) {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    let manifest: toml::Value = match toml::from_str(&contents) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let profile_release = match manifest.get("profile").and_then(|p| p.get("release")) {
        Some(t) => t,
        None => {
            // No [profile.release] at all — warn for all three settings.
            return vec![
                Check {
                    name: "release opt-level",
                    status: Status::Warn,
                    detail: "opt-level not set in [profile.release]".into(),
                    fix: Some(r#"add to Cargo.toml: [profile.release]\nopt-level = "z""#),
                },
                Check {
                    name: "release lto",
                    status: Status::Warn,
                    detail: "lto not set in [profile.release]".into(),
                    fix: Some("add to Cargo.toml: [profile.release]\nlto = true"),
                },
                Check {
                    name: "release codegen-units",
                    status: Status::Warn,
                    detail: "codegen-units not set in [profile.release]".into(),
                    fix: Some("add to Cargo.toml: [profile.release]\ncodegen-units = 1"),
                },
            ];
        }
    };

    let mut checks = Vec::new();

    // opt-level = "z"
    let opt_ok = profile_release
        .get("opt-level")
        .and_then(|v| v.as_str())
        .map(|s| s == "z")
        .unwrap_or(false);
    if !opt_ok {
        checks.push(Check {
            name: "release opt-level",
            status: Status::Warn,
            detail: "opt-level is not \"z\" in [profile.release]".into(),
            fix: Some(r#"set in Cargo.toml: [profile.release]\nopt-level = "z""#),
        });
    }

    // lto = true
    let lto_ok = profile_release
        .get("lto")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !lto_ok {
        checks.push(Check {
            name: "release lto",
            status: Status::Warn,
            detail: "lto is not true in [profile.release]".into(),
            fix: Some("set in Cargo.toml: [profile.release]\nlto = true"),
        });
    }

    // codegen-units = 1
    let cgu_ok = profile_release
        .get("codegen-units")
        .and_then(|v| v.as_integer())
        .map(|n| n == 1)
        .unwrap_or(false);
    if !cgu_ok {
        checks.push(Check {
            name: "release codegen-units",
            status: Status::Warn,
            detail: "codegen-units is not 1 in [profile.release]".into(),
            fix: Some("set in Cargo.toml: [profile.release]\ncodegen-units = 1"),
        });
    }

    checks
}

/// Whether git excludes the project's `Cargo.lock` from version control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockIgnored {
    /// An ignore rule matches the lockfile and nothing has committed it.
    Yes,
    /// No ignore rule excludes it — including the case of a lockfile already
    /// tracked, which git carries regardless of a matching rule.
    No,
    /// Could not tell: git is missing, or this is not a repository.
    Unknown,
}

/// Classify a `Cargo.lock` probe into a report line.
///
/// A committed lockfile is what makes CI resolve the same dependency versions
/// the contract was tested and audited against; without one, every build picks
/// up whatever has been published since. That is a [`Status::Warn`] rather
/// than a [`Status::Fail`] — the project still builds, just not reproducibly.
///
/// `present` is whether the file exists on disk; `ignored` is git's verdict on
/// whether version control would carry it.
pub fn classify_cargo_lock(present: bool, ignored: LockIgnored) -> Check {
    match (present, ignored) {
        (true, LockIgnored::Yes) => Check {
            name: "Cargo.lock",
            status: Status::Warn,
            detail: "present but excluded by .gitignore — CI never sees it".into(),
            fix: Some(
                "remove the Cargo.lock entry from .gitignore, then: \
                 git add -f Cargo.lock && git commit -m \"commit Cargo.lock\"",
            ),
        },
        (true, LockIgnored::No) => Check {
            name: "Cargo.lock",
            status: Status::Pass,
            detail: "present, and no ignore rule excludes it".into(),
            fix: None,
        },
        (true, LockIgnored::Unknown) => Check {
            name: "Cargo.lock",
            status: Status::Pass,
            detail: "present".into(),
            fix: None,
        },
        (false, LockIgnored::Yes) => Check {
            name: "Cargo.lock",
            status: Status::Warn,
            detail: "not found, and .gitignore excludes it".into(),
            fix: Some(
                "remove the Cargo.lock entry from .gitignore, then: \
                 cargo generate-lockfile && git add Cargo.lock",
            ),
        },
        (false, _) => Check {
            name: "Cargo.lock",
            status: Status::Warn,
            detail: "not found — CI resolves fresh dependency versions".into(),
            fix: Some("cargo generate-lockfile, then commit Cargo.lock"),
        },
    }
}

/// Ask git whether an ignore rule excludes `Cargo.lock` in `project_dir`.
///
/// `git check-ignore --quiet` answers in its exit status: 0 when a rule
/// matches, 1 when none does, 128 outside a repository. A tracked file counts
/// as *not* ignored even when a rule matches it, which is exactly the question
/// asked here — a lockfile already in the index is version-controlled whatever
/// `.gitignore` says.
fn lock_ignored(project_dir: &Path) -> LockIgnored {
    let output = std::process::Command::new("git")
        .args(["check-ignore", "--quiet", "Cargo.lock"])
        .current_dir(project_dir)
        .output();
    match output {
        Ok(o) => match o.status.code() {
            Some(0) => LockIgnored::Yes,
            Some(1) => LockIgnored::No,
            _ => LockIgnored::Unknown,
        },
        Err(_) => LockIgnored::Unknown,
    }
}

/// Report whether the project has a `Cargo.lock` that version control carries.
///
/// Returns `None` (no report line at all) when `project_dir` is not a cargo
/// project — there is no lockfile to expect.
///
/// Thin system-touching wrapper around [`classify_cargo_lock`].
pub fn cargo_lock_check(project_dir: &Path) -> Option<Check> {
    if !project_dir.join("Cargo.toml").is_file() {
        return None;
    }
    let present = project_dir.join("Cargo.lock").is_file();
    Some(classify_cargo_lock(present, lock_ignored(project_dir)))
}

/// Run a fast `cargo build --target wasm32v1-none` in `project_dir` and
/// report whether it succeeds, with timing.
///
/// Returns `None` (no report line at all) when `project_dir` does not look
/// like a cargo project — there is nothing to smoke-build.
///
/// Thin system-touching wrapper; not unit-tested beyond the "not a project"
/// case.
pub fn wasm_build_check(project_dir: &Path) -> Option<Check> {
    if !project_dir.join("Cargo.toml").is_file() {
        return None;
    }

    let start = std::time::Instant::now();
    let output = std::process::Command::new("cargo")
        .args(["build", "--target", "wasm32v1-none"])
        .current_dir(project_dir)
        .output();
    let elapsed_ms = start.elapsed().as_millis();

    Some(match output {
        Ok(o) if o.status.success() => Check {
            name: "wasm build",
            status: Status::Pass,
            detail: format!("builds to wasm32v1-none ({elapsed_ms} ms)"),
            fix: None,
        },
        Ok(_) => Check {
            name: "wasm build",
            status: Status::Fail,
            detail: format!("cargo build --target wasm32v1-none failed ({elapsed_ms} ms)"),
            fix: Some("run `cargo build --target wasm32v1-none` directly to see the error"),
        },
        Err(_) => Check {
            name: "wasm build",
            status: Status::Fail,
            detail: "could not run cargo".into(),
            fix: Some("install Rust: https://rustup.rs"),
        },
    })
}

/// Run all environment checks.
pub fn run_checks() -> Vec<Check> {
    run_checks_with_network(true)
}

/// Run environment checks, optionally omitting the RPC connectivity probe.
pub fn run_checks_with_network(allow_network: bool) -> Vec<Check> {
    let mut checks = Vec::new();

    // rustc, with a minimum version.
    let rustup_present = rustup_available();
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
            fix: Some(rust_update_fix(rustup_present)),
        },
        None => Check {
            name: "rustc",
            status: Status::Fail,
            detail: "not found".into(),
            fix: Some(rust_install_fix(rustup_present)),
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
            fix: Some(rust_install_fix(rustup_present)),
        },
    });

    // wasm32-unknown-unknown target (issue #45).
    //
    // Some toolchains and projects still require the older `wasm32-unknown-unknown`
    // target alongside the newer `wasm32v1-none`. Check for it explicitly.
    {
        let installed_targets_wasm32 = if rustup_present {
            std::process::Command::new("rustup")
                .args(["target", "list", "--installed"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        } else {
            None
        };
        checks.push(match installed_targets_wasm32 {
            Some(targets) if targets.lines().any(|t| t.trim() == "wasm32-unknown-unknown") => {
                Check {
                    name: "wasm32-unknown-unknown",
                    status: Status::Pass,
                    detail: "installed".into(),
                    fix: None,
                }
            }
            Some(_) => Check {
                name: "wasm32-unknown-unknown",
                status: Status::Fail,
                detail: "missing wasm32 target".into(),
                fix: missing_target_fix("wasm32-unknown-unknown", rustup_present),
            },
            None => Check {
                name: "wasm32-unknown-unknown",
                status: Status::Warn,
                detail: "rustup not found — could not verify".into(),
                fix: missing_target_fix("wasm32-unknown-unknown", false),
            },
        });
    }

    // stellar-cli — presence, minimum version and known-bad releases.
    checks.push(match capture("stellar", &["--version"]) {
        Some(line) if version_at_least(&line, MIN_STELLAR) => {
            if let Some(recommended) = known_broken_stellar_cli_replacement(&line) {
                Check {
                    name: "stellar-cli",
                    status: Status::Warn,
                    detail: format!("{line} (known-broken release; upgrade to {recommended})"),
                    fix: Some(
                        "upgrade: cargo install --locked stellar-cli  (or: brew upgrade stellar-cli)",
                    ),
                }
            } else {
                Check {
                    name: "stellar-cli",
                    status: Status::Pass,
                    detail: line,
                    fix: None,
                }
            }
        }
        Some(line) => Check {
            name: "stellar-cli",
            status: Status::Warn,
            detail: format!("{line} (need >= {}.{}.0)", MIN_STELLAR.0, MIN_STELLAR.1),
            fix: Some(
                "upgrade: cargo install --locked stellar-cli  (or: brew upgrade stellar-cli)",
            ),
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

    // Testnet RPC connectivity (issue #46).
    if allow_network {
        checks.push(rpc_connectivity_check(TESTNET_RPC_URL));
    }

    // git — recommended only.
    let git_version = capture("git", &["--version"]);
    checks.push(match &git_version {
        Some(line) => Check {
            name: "git",
            status: Status::Pass,
            detail: line.clone(),
            fix: None,
        },
        None => Check {
            name: "git",
            status: Status::Warn,
            detail: "not found".into(),
            fix: Some("install git: https://git-scm.com/downloads"),
        },
    });

    // git committer identity (issue #71) — only meaningful when git exists.
    if git_version.is_some() {
        checks.push(git_identity_check());
    }

    // Docker (issue #70) — optional, used for reproducible wasm builds.
    checks.push(docker_check());

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

/// Render the report as JSON.
pub fn format_json_report(checks: &[Check]) -> String {
    serde_json::to_string_pretty(checks).unwrap_or_else(|_| "[]".to_string())
}

/// Count required checks that failed.
pub fn failure_count(checks: &[Check]) -> usize {
    checks
        .iter()
        .filter(|check| check.status == Status::Fail)
        .count()
}

// ---------------------------------------------------------------------------
// Auto-fix (`--fix`)
// ---------------------------------------------------------------------------

/// An auto-installable remedy for a failing check: a concrete, non-interactive
/// command doctor can run to resolve it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remedy {
    /// The name of the [`Check`] this remedy resolves.
    pub check: &'static str,
    /// The program to invoke, e.g. `rustup` or `cargo`.
    pub program: &'static str,
    /// Arguments passed to `program`.
    pub args: Vec<String>,
}

impl Remedy {
    /// The remedy rendered as a copy-pasteable shell command line.
    pub fn command_line(&self) -> String {
        if self.args.is_empty() {
            self.program.to_string()
        } else {
            format!("{} {}", self.program, self.args.join(" "))
        }
    }
}

/// The auto-installable remedy for a check, when one exists.
///
/// Only genuinely runnable, non-interactive install commands are returned, and
/// only for checks that are currently [`Status::Fail`]. Checks whose fix is a
/// URL or a manual step (`rustc`/`cargo` install, `git`, `soroban-sdk`
/// version, or the target when `rustup` itself is missing) return `None`, so
/// they are reported rather than auto-fixed.
pub fn remedy(check: &Check) -> Option<Remedy> {
    if check.status != Status::Fail {
        return None;
    }
    match check.name {
        "wasm32v1-none target" => {
            if !rustup_available() {
                return None;
            }
            let args = check
                .detail
                .split("exact fix: ")
                .last()
                .filter(|cmd| cmd.starts_with("rustup "))
                .map(|cmd| {
                    cmd.split_whitespace()
                        .skip(1)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(|| vec!["target".into(), "add".into(), "wasm32v1-none".into()]);
            Some(Remedy {
                check: check.name,
                program: "rustup",
                args,
            })
        }
        "stellar-cli" => Some(Remedy {
            check: check.name,
            program: "cargo",
            args: vec!["install".into(), "--locked".into(), "stellar-cli".into()],
        }),
        _ => None,
    }
}

/// Every auto-fixable remedy for the current set of checks, in check order.
pub fn fixable_remedies(checks: &[Check]) -> Vec<Remedy> {
    checks.iter().filter_map(remedy).collect()
}

/// The result of attempting a single [`Remedy`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixOutcome {
    /// The name of the check this remedy targeted.
    pub check: &'static str,
    /// The command line that was run.
    pub command: String,
    /// Whether the command exited successfully.
    pub succeeded: bool,
    /// Failure detail (non-zero status or spawn error), when it failed.
    pub detail: Option<String>,
}

/// The plan shown to the user before confirmation: the commands `--fix` will
/// run, one per line.
pub fn format_fix_plan(remedies: &[Remedy]) -> String {
    let mut out = String::from("The following command(s) will be run:\n\n");
    for remedy in remedies {
        out.push_str(&format!("  {}\n", remedy.command_line()));
    }
    out.push('\n');
    out
}

/// A short human summary of what `--fix` ran and whether each command worked.
pub fn format_fix_summary(outcomes: &[FixOutcome]) -> String {
    let mut out = String::from("\nfix results:\n");
    for outcome in outcomes {
        let symbol = if outcome.succeeded { "✓" } else { "✗" };
        out.push_str(&format!("  {symbol} {}", outcome.command));
        if let Some(detail) = &outcome.detail {
            out.push_str(&format!(" — {detail}"));
        }
        out.push('\n');
    }
    out.push('\n');
    out
}

/// Run a single remedy, inheriting stdio so the user sees install progress.
///
/// Thin system-touching wrapper (like [`capture`]); not unit-tested.
fn run_remedy(remedy: &Remedy) -> FixOutcome {
    let command = remedy.command_line();
    let status = std::process::Command::new(remedy.program)
        .args(remedy.args.iter().map(String::as_str))
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status();
    match status {
        Ok(s) if s.success() => FixOutcome {
            check: remedy.check,
            command,
            succeeded: true,
            detail: None,
        },
        Ok(s) => FixOutcome {
            check: remedy.check,
            command,
            succeeded: false,
            detail: Some(match s.code() {
                Some(code) => format!("exited with status {code}"),
                None => "terminated by signal".to_string(),
            }),
        },
        Err(e) => FixOutcome {
            check: remedy.check,
            command,
            succeeded: false,
            detail: Some(e.to_string()),
        },
    }
}

/// Run every remedy in order, returning one outcome per remedy.
///
/// Thin system-touching wrapper; not unit-tested.
fn apply_remedies(remedies: &[Remedy]) -> Vec<FixOutcome> {
    remedies.iter().map(run_remedy).collect()
}

/// Prompt on stdout and read a yes/no answer from stdin. Anything other than
/// `y`/`yes` (case-insensitive), including EOF, is treated as "no".
///
/// Thin system-touching wrapper; not unit-tested.
fn confirm(prompt: &str) -> bool {
    use std::io::Write;
    print!("{prompt} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

fn config_network_url(config: Option<&soroban_forge_core::config::ForgeConfig>) -> Option<String> {
    let config = config?;
    if let Some(url) = config.network.rpc_url.as_deref() {
        return Some(url.to_owned());
    }
    let name = config.network.name.as_deref()?;
    // Resolve well-known names through the network crate so doctor's mapping
    // cannot drift from the RPC URLs the rest of the toolchain uses (#485).
    // Unknown names still fall through to the literal string, which is the
    // only thing available for a custom network with no explicit rpc_url.
    match soroban_forge_network::well_known(name) {
        Some(network) => Some(network.rpc_url),
        None => Some(name.to_string()),
    }
}

fn normalize_check_name(name: &str) -> String {
    name.to_ascii_lowercase()
        .replace(" ", "-")
        .replace("_", "-")
}

fn list_check_names() -> Vec<&'static str> {
    vec![
        "rustc",
        "cargo",
        "wasm32v1-none-target",
        "wasm32-unknown-unknown",
        "stellar-cli",
        "testnet-rpc",
        "git",
        "git-identity",
        "docker",
        "toolchain",
        "soroban-sdk",
        "release-opt-level",
        "release-lto",
        "release-codegen-units",
        "disk-space",
    ]
}

/// The `doctor` subcommand.
pub struct DoctorPlugin;

impl DoctorPlugin {
    /// Run every check, including the project-local `soroban-sdk` check when
    /// invoked inside a contract project.
    ///
    /// `do_build` opts into the `cargo build --target wasm32v1-none` smoke
    /// check (issue #72), which is otherwise skipped since it is much slower
    /// than the rest of the report.
    fn gather_checks(&self, ctx: &ForgeContext, do_build: bool) -> Vec<Check> {
        let mut checks = Vec::new();
        checks.push(disk_space_check(&ctx.cwd));
        if ctx.offline {
            checks.push(Check {
                name: "testnet RPC",
                status: Status::Warn,
                detail: "skipped (--offline)".into(),
                fix: None,
            });
        } else {
            let url = config_network_url(ctx.config.as_ref())
                .unwrap_or_else(|| TESTNET_RPC_URL.to_string());
            checks.push(rpc_connectivity_check(&url));
        }
        checks.extend(run_checks_with_network(false));
        checks.push(toolchain_check(&ctx.cwd)); // issue #109
        checks.push(wasm32_target_check(&ctx.cwd)); // issue #251
        if let Some(check) = sdk_version_check_with(&ctx.cwd, || {
            if ctx.offline {
                None
            } else {
                latest_published_sdk_version()
            }
        }) {
            checks.push(check);
        }
        // A committed lockfile, without which CI cannot reproduce a build.
        if let Some(check) = cargo_lock_check(&ctx.cwd) {
            checks.push(check);
        }
        // Release profile size-optimisation checks (issue #48).
        checks.extend(release_profile_checks(&ctx.cwd));
        if do_build {
            if let Some(check) = wasm_build_check(&ctx.cwd) {
                checks.push(check);
            }
        }
        checks
    }

    /// Print the report as text or JSON, honouring `--quiet`.
    fn emit_report(&self, checks: &[Check], use_json: bool, quiet: bool) {
        if use_json {
            println!("{}", format_json_report(checks));
        } else if !quiet {
            print!("{}", format_report(checks));
        }
    }

    /// Decide whether to run the remedies.
    ///
    /// `--yes` proceeds unconditionally. In JSON mode we never prompt (it
    /// would corrupt the machine-readable stream), so a fix only proceeds
    /// there when `--yes` is given. Otherwise we print the plan and ask.
    fn confirm_fix(
        &self,
        remedies: &[Remedy],
        use_json: bool,
        assume_yes: bool,
        quiet: bool,
    ) -> bool {
        if assume_yes {
            return true;
        }
        if use_json {
            return false;
        }
        if !quiet {
            print!("{}", format_fix_plan(remedies));
        }
        confirm("Proceed?")
    }
}

impl ForgePlugin for DoctorPlugin {
    fn name(&self) -> &'static str {
        "doctor"
    }

    fn command(&self) -> Command {
        Command::new("doctor")
            .about(
                "Check that Rust, the wasm32v1-none target and stellar-cli are installed, \
                 and that the project's soroban-sdk is up to date",
            )
            .arg(
                Arg::new("json")
                    .long("json")
                    .action(ArgAction::SetTrue)
                    .help("Output check results as JSON"),
            )
            .arg(Arg::new("fix").long("fix").action(ArgAction::SetTrue).help(
                "Attempt to install missing toolchain components \
                         (rustup target add, cargo install), then re-check",
            ))
            .arg(
                Arg::new("yes")
                    .long("yes")
                    .short('y')
                    .action(ArgAction::SetTrue)
                    .help("Assume \"yes\"; run --fix remedies without prompting"),
            )
            .arg(
                Arg::new("build")
                    .long("build")
                    .action(ArgAction::SetTrue)
                    .help(
                        "Also run a smoke-build (`cargo build --target wasm32v1-none`) \
                         of the current project and report success/failure with timing",
                    ),
            )
            .arg(
                Arg::new("check")
                    .long("check")
                    .value_name("NAME")
                    .help("Run only the named check (use --list-checks to see valid choices)"),
            )
            .arg(
                Arg::new("list-checks")
                    .long("list-checks")
                    .action(ArgAction::SetTrue)
                    .help("Print the available check names and exit"),
            )
    }

    fn run(&self, matches: &ArgMatches, ctx: &ForgeContext) -> Result<()> {
        let use_json = ctx.json || matches.get_flag("json");
        let do_fix = matches.get_flag("fix");
        let do_build = matches.get_flag("build");
        let assume_yes = ctx.yes || matches.get_flag("yes");
        let selected_check = matches.get_one::<String>("check").map(String::as_str);
        let list_checks = matches.get_flag("list-checks");

        if list_checks {
            println!("{}", list_check_names().join("\n"));
            return Ok(());
        }

        if let Some(selected) = selected_check {
            let normalized = normalize_check_name(selected);
            if !list_check_names().iter().any(|n| normalize_check_name(n) == normalized) {
                return Err(ForgeError::InvalidArgument(format!(
                    "unknown doctor check `{selected}` (valid: {})",
                    list_check_names().join(", ")
                )));
            }
        }

        if ctx.offline && do_fix {
            return Err(ForgeError::InvalidArgument(
                "doctor --fix is unavailable in offline mode because remedies may download tools"
                    .into(),
            ));
        }

        let mut checks = self.gather_checks(ctx, do_build);
        if let Some(selected) = selected_check {
            checks = checks
                .into_iter()
                .filter(|check| normalize_check_name(check.name) == normalize_check_name(selected))
                .collect();
        }

        if do_fix {
            let remedies = fixable_remedies(&checks);
            if !remedies.is_empty() && self.confirm_fix(&remedies, use_json, assume_yes, ctx.quiet)
            {
                let outcomes = apply_remedies(&remedies);
                if !use_json && !ctx.quiet {
                    print!("{}", format_fix_summary(&outcomes));
                }
                // Re-check so the final report reflects the fixes; any
                // non-fixable issues (and any remedy that failed) remain.
                checks = self.gather_checks(ctx, do_build);
                if let Some(selected) = selected_check {
                    checks = checks
                        .into_iter()
                        .filter(|check| normalize_check_name(check.name) == normalize_check_name(selected))
                        .collect();
                }
            }
        }

        self.emit_report(&checks, use_json, ctx.quiet);

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
    use soroban_forge_core::config::{ForgeConfig, NetworkConfig};

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

    #[test]
    fn missing_legacy_wasm_target_does_not_fail_the_run() {
        // The legacy wasm32-unknown-unknown target is optional (#484): a project
        // that only needs wasm32v1-none must not get a non-zero exit just because
        // the older target is absent.
        let legacy_missing = Check {
            name: "wasm32-unknown-unknown",
            status: Status::Warn,
            detail: "missing optional legacy wasm32 target".into(),
            fix: Some("rustup target add wasm32-unknown-unknown"),
        };
        let required_present = Check {
            name: "wasm32v1-none-target",
            status: Status::Pass,
            detail: "installed".into(),
            fix: None,
        };

        let checks = [required_present, legacy_missing];

        assert_eq!(
            failure_count(&checks),
            0,
            "a missing optional legacy target must not count as a failure"
        );

        // The report still mentions the warning, but it must not be reported as
        // a failure - the warning is informational, not blocking.
        let report = format_report(&checks);
        assert!(
            report.contains("0 failure(s), 1 warning(s)"),
            "the legacy target must be reported as a warning, not a failure: {report}"
        );
        assert!(
            !report.contains('\u{2717}'),
            "no check may render as a failure marker: {report}"
        );
    }

    // ---- wasm smoke-build check ----

    #[test]
    fn wasm_build_check_skipped_outside_a_project() {
        let dir = tempfile::tempdir().unwrap();
        assert!(wasm_build_check(dir.path()).is_none());
    }

    // ---- soroban-sdk version check ----

    fn project_with_manifest(manifest: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), manifest).unwrap();
        dir
    }

    fn sdk_check_at_template_pin(project_dir: &Path) -> Option<Check> {
        sdk_version_check_with(project_dir, || Some(SOROBAN_SDK_VERSION.into()))
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
    fn known_bad_stellar_cli_versions_warn_with_recommendation() {
        assert_eq!(known_broken_stellar_cli_replacement("stellar-cli 27.0.0"), Some("27.0.1"));
        assert_eq!(known_broken_stellar_cli_replacement("stellar-cli 28.0.0"), Some("28.0.1"));
        assert_eq!(known_broken_stellar_cli_replacement("stellar-cli 26.1.0"), None);
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
        assert!(sdk_check_at_template_pin(dir.path()).is_none());
    }

    #[test]
    fn no_op_without_soroban_sdk_dependency() {
        let dir = project_with_manifest(
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n\n[dependencies]\nserde = \"1\"\n",
        );
        assert!(sdk_check_at_template_pin(dir.path()).is_none());
    }

    #[test]
    fn warns_when_project_sdk_is_behind() {
        let dir = project_with_manifest(
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n\n[dependencies]\nsoroban-sdk = \"25.0.0\"\n",
        );
        let check = sdk_check_at_template_pin(dir.path()).unwrap();
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
        let check = sdk_check_at_template_pin(dir.path()).unwrap();
        assert_eq!(check.status, Status::Pass);
        assert!(check
            .detail
            .contains(&format!("latest: {SOROBAN_SDK_VERSION}")));
        assert!(check.fix.is_none());
    }

    #[test]
    fn passes_when_project_sdk_is_ahead() {
        let dir = project_with_manifest(
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n\n[dependencies]\nsoroban-sdk = \"99.0.0\"\n",
        );
        assert_eq!(
            sdk_check_at_template_pin(dir.path()).unwrap().status,
            Status::Pass
        );
    }

    #[test]
    fn reads_table_form_and_dev_dependencies() {
        let table = project_with_manifest(
            "[dependencies]\nsoroban-sdk = { version = \"25.1.2\", features = [\"testutils\"] }\n",
        );
        let check = sdk_check_at_template_pin(table.path()).unwrap();
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("25.1.2"));

        let dev = project_with_manifest(&format!(
            "[dev-dependencies]\nsoroban-sdk = \"{SOROBAN_SDK_VERSION}\"\n"
        ));
        assert_eq!(
            sdk_check_at_template_pin(dev.path()).unwrap().status,
            Status::Pass
        );
    }

    #[test]
    fn warns_on_versionless_dependency() {
        let dir = project_with_manifest(
            "[dependencies]\nsoroban-sdk = { git = \"https://example.com/sdk\" }\n",
        );
        let check = sdk_check_at_template_pin(dir.path()).unwrap();
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("no version specified"));
    }

    #[test]
    fn config_network_url_resolves_mainnet_to_a_real_url() {
        let config = ForgeConfig {
            network: NetworkConfig {
                name: Some("mainnet".into()),
                rpc_url: None,
                passphrase: None,
            },
            ..Default::default()
        };
        let url = config_network_url(Some(&config)).expect("mainnet must resolve");
        assert_ne!(
            url, "mainnet",
            "mainnet must not resolve to the literal network name (#485)"
        );
        assert!(
            url.starts_with("http"),
            "mainnet must resolve to a real RPC URL, got {url:?}"
        );
    }

    #[test]
    fn config_network_url_matches_network_crate_for_well_known_names() {
        for name in ["testnet", "futurenet", "mainnet", "localnet"] {
            let config = ForgeConfig {
                network: NetworkConfig {
                    name: Some(name.into()),
                    rpc_url: None,
                    passphrase: None,
                },
                ..Default::default()
            };
            let url = config_network_url(Some(&config)).expect("well-known name must resolve");
            let expected = soroban_forge_network::well_known(name)
                .expect("network crate must know this name")
                .rpc_url;
            assert_eq!(
                url, expected,
                "doctor and the network crate must not drift for {name:?}"
            );
        }
    }

    #[test]
    fn config_network_url_prefers_explicit_rpc_url() {
        let config = ForgeConfig {
            network: NetworkConfig {
                name: Some("mainnet".into()),
                rpc_url: Some("https://example.com/custom".into()),
                passphrase: None,
            },
            ..Default::default()
        };
        assert_eq!(
            config_network_url(Some(&config)).as_deref(),
            Some("https://example.com/custom")
    fn warns_when_project_sdk_is_behind_latest_published() {
        let dir = project_with_manifest(
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n\n[dependencies]\nsoroban-sdk = \"26.1.0\"\n",
        );
        let check = sdk_version_check_with(dir.path(), || Some("27.0.0".into())).unwrap();
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("soroban-sdk 26.1.0"));
        assert!(check.detail.contains("latest: 27.0.0"));
        assert_eq!(
            check.fix,
            Some("update the soroban-sdk version in Cargo.toml")
        );
    }

    #[test]
    fn config_network_url_keeps_unknown_names_as_literals() {
        let config = ForgeConfig {
            network: NetworkConfig {
                name: Some("my-private-net".into()),
                rpc_url: None,
                passphrase: None,
            },
            ..Default::default()
        };
        assert_eq!(
            config_network_url(Some(&config)).as_deref(),
            Some("my-private-net"),
            "an unknown name with no rpc_url has nothing else to fall back to"
        );
    fn falls_back_to_template_pin_when_registry_is_unavailable() {
        let dir = project_with_manifest(&format!(
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n\n[dependencies]\nsoroban-sdk = \"{SOROBAN_SDK_VERSION}\"\n",
        ));
        let check = sdk_version_check_with(dir.path(), || None).unwrap();
        assert_eq!(check.status, Status::Pass);
        assert!(check.detail.contains("latest published unavailable"));
        assert!(check.detail.contains(SOROBAN_SDK_VERSION));
    }

    #[test]
    fn json_report_formatting() {
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
        let json_str = format_json_report(&checks);
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert!(parsed.is_array());
        assert_eq!(parsed[0]["name"], "rustc");
        assert_eq!(parsed[0]["status"], "pass");
        assert_eq!(parsed[0]["detail"], "rustc 1.90.0");
        assert!(parsed[0]["fix"].is_null());
        assert_eq!(parsed[1]["name"], "stellar-cli");
        assert_eq!(parsed[1]["status"], "fail");
        assert_eq!(parsed[1]["fix"], "install: brew install stellar-cli");
    }

    // ---- docker (issue #70) ----

    #[test]
    fn docker_present_and_running_passes() {
        let check = classify_docker(Some("Docker version 27.3.1, build ce12230"), true);
        assert_eq!(check.status, Status::Pass);
        assert!(check.detail.contains("27.3.1"));
        assert!(check.fix.is_none());
    }

    #[test]
    fn docker_installed_but_daemon_down_warns() {
        let check = classify_docker(Some("Docker version 27.3.1"), false);
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("daemon is not responding"));
        assert!(check.fix.unwrap().contains("start Docker"));
    }

    #[test]
    fn docker_absent_warns_without_failing() {
        let check = classify_docker(None, false);
        assert_eq!(check.status, Status::Warn);
        assert_eq!(failure_count(std::slice::from_ref(&check)), 0);
        assert!(check.detail.contains("not found"));
        assert!(check.fix.unwrap().contains("docs.docker.com"));
    }

    // ---- active toolchain (issue #109) ----

    #[test]
    fn toolchain_reports_stable_channel() {
        let check = classify_toolchain(Some("stable-x86_64-unknown-linux-gnu (default)"), None);
        assert_eq!(check.status, Status::Pass);
        assert!(check.detail.contains("stable-x86_64-unknown-linux-gnu"));
        assert!(check.detail.contains("channel: stable"));
        assert!(check.fix.is_none());
    }

    #[test]
    fn toolchain_reports_nightly_channel() {
        let check = classify_toolchain(
            Some("nightly-x86_64-pc-windows-msvc (overridden by '/proj/rust-toolchain.toml')"),
            None,
        );
        assert!(check.detail.contains("channel: nightly"));
    }

    #[test]
    fn fixable_remedies_skip_already_fixed_checks() {
        let checks = vec![
            Check {
                name: "wasm32v1-none target",
                status: Status::Pass,
                detail: "installed".into(),
                fix: None,
            },
            Check {
                name: "stellar-cli",
                status: Status::Fail,
                detail: "not found".into(),
                fix: Some("install: brew install stellar-cli"),
            },
        ];

        let remedies = fixable_remedies(&checks);
        assert_eq!(remedies.len(), 1);
        assert_eq!(remedies[0].check, "stellar-cli");
    }

    #[test]
    fn partial_fix_keeps_remaining_failures_for_re_run() {
        let checks = vec![
            Check {
                name: "wasm32v1-none target",
                status: Status::Fail,
                detail: "not installed".into(),
                fix: Some("rustup target add wasm32v1-none"),
            },
            Check {
                name: "stellar-cli",
                status: Status::Fail,
                detail: "not found".into(),
                fix: Some("install: brew install stellar-cli"),
            },
        ];

        let first = fixable_remedies(&checks);
        if rustup_available() {
            assert_eq!(first.len(), 2);
        } else {
            assert_eq!(first.len(), 1);
            assert_eq!(first[0].check, "stellar-cli");
        }

        let next = vec![
            Check {
                name: "wasm32v1-none target",
                status: Status::Pass,
                detail: "installed".into(),
                fix: None,
            },
            Check {
                name: "stellar-cli",
                status: Status::Fail,
                detail: "not found".into(),
                fix: Some("install: brew install stellar-cli"),
            },
        ];

        let remaining = fixable_remedies(&next);
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].check, "stellar-cli");
    }

    #[test]
    fn toolchain_reports_pinned_version_when_not_a_named_channel() {
        let check = classify_toolchain(Some("1.84.0-x86_64-unknown-linux-gnu"), None);
        assert!(check.detail.contains("channel: pinned"));
    }

    #[test]
    fn toolchain_falls_back_to_rustc_version_without_rustup() {
        let check = classify_toolchain(None, Some("rustc 1.90.0-nightly (abc 2026-01-01)"));
        assert_eq!(check.status, Status::Pass);
        assert!(check.detail.contains("channel: nightly"));
        assert!(check.detail.contains("rustup not found"));
    }

    #[test]
    fn toolchain_warns_when_neither_tool_is_found() {
        let check = classify_toolchain(None, None);
        assert_eq!(check.status, Status::Warn);
        assert!(check.fix.is_some());
    }

    #[test]
    fn missing_target_uses_manual_instructions_without_rustup() {
        let fix = missing_target_fix("wasm32v1-none", false);
        assert_eq!(
            fix,
            Some(
                "install the wasm32v1-none target via your system package manager or the Rust distribution you installed; rustup is not available on PATH"
            )
        );
        assert!(!fix.unwrap().contains("rustup"));
    }

    #[test]
    fn toolchain_is_not_auto_fixable() {
        assert!(remedy(&fail("toolchain")).is_none());
    }

    // ---- git identity (issue #71) ----

    #[test]
    fn git_identity_set_passes() {
        let check = classify_git_identity(Some("Ada Lovelace"), Some("ada@example.com"));
        assert_eq!(check.status, Status::Pass);
        assert_eq!(check.detail, "Ada Lovelace <ada@example.com>");
        assert!(check.fix.is_none());
    }

    #[test]
    fn git_identity_missing_warns_with_commands() {
        let check = classify_git_identity(None, None);
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("user.name and user.email"));
        let fix = check.fix.unwrap();
        assert!(fix.contains("git config --global user.name"));
        assert!(fix.contains("git config --global user.email"));
    }

    #[test]
    fn git_identity_reports_which_half_is_missing() {
        let no_email = classify_git_identity(Some("Ada"), None);
        assert_eq!(no_email.status, Status::Warn);
        assert_eq!(no_email.detail, "user.email is not set");

        let no_name = classify_git_identity(None, Some("ada@example.com"));
        assert_eq!(no_name.detail, "user.name is not set");
    }

    #[test]
    fn blank_git_identity_counts_as_unset() {
        let check = classify_git_identity(Some("  "), Some(""));
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("user.name and user.email"));
    }

    #[test]
    fn docker_and_git_identity_and_disk_space_are_not_auto_fixable() {
        for name in ["docker", "git identity", "disk space"] {
            assert!(remedy(&fail(name)).is_none(), "{name}");
        }
    }

    // ---- Cargo.lock (reproducible CI builds) ----

    /// A cargo project inside its own git repository, so `git check-ignore`
    /// answers from this project's rules rather than an enclosing repo's.
    /// `None` when git is unavailable, leaving the caller nothing to probe.
    fn git_project(manifest: &str) -> Option<tempfile::TempDir> {
        let dir = project_with_manifest(manifest);
        let initialized = std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(dir.path())
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        initialized.then_some(dir)
    }

    #[test]
    fn lockfile_present_and_not_ignored_passes() {
        let check = classify_cargo_lock(true, LockIgnored::No);
        assert_eq!(check.status, Status::Pass);
    // ---- disk space check ----

    #[test]
    fn format_bytes_formats_correct_units() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(500 * 1024 * 1024), "500.0 MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0 GB");
        assert_eq!(format_bytes(50 * 1024 * 1024 * 1024), "50.0 GB");
        assert_eq!(format_bytes(2 * 1024 * 1024 * 1024 * 1024), "2.0 TB");
    }

    #[test]
    fn disk_space_reports_pass_when_above_threshold() {
        let check = classify_disk_space(Some(10 * 1024 * 1024 * 1024), MIN_FREE_DISK_SPACE_BYTES);
        assert_eq!(check.status, Status::Pass);
        assert_eq!(check.name, "disk space");
        assert!(check.detail.contains("10.0 GB free"));
        assert!(check.fix.is_none());
    }

    #[test]
    fn lockfile_present_without_a_git_verdict_passes() {
        // No git, or not a repository: there is a lockfile, and nothing
        // suggests version control would drop it.
        let check = classify_cargo_lock(true, LockIgnored::Unknown);
        assert_eq!(check.status, Status::Pass);
    fn disk_space_reports_pass_at_exact_threshold() {
        let check = classify_disk_space(Some(MIN_FREE_DISK_SPACE_BYTES), MIN_FREE_DISK_SPACE_BYTES);
        assert_eq!(check.status, Status::Pass);
        assert_eq!(check.name, "disk space");
        assert!(check.detail.contains("1.0 GB free"));
        assert!(check.fix.is_none());
    }

    #[test]
    fn gitignored_lockfile_warns() {
        let check = classify_cargo_lock(true, LockIgnored::Yes);
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("excluded by .gitignore"));
        assert!(check.fix.unwrap().contains(".gitignore"));
    }

    #[test]
    fn missing_lockfile_warns() {
        let check = classify_cargo_lock(false, LockIgnored::No);
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("not found"));
        assert!(check.fix.unwrap().contains("cargo generate-lockfile"));
    }

    #[test]
    fn missing_and_gitignored_lockfile_reports_both() {
        let check = classify_cargo_lock(false, LockIgnored::Yes);
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("not found"));
        assert!(check.detail.contains(".gitignore"));
        let fix = check.fix.unwrap();
        assert!(fix.contains(".gitignore"));
        assert!(fix.contains("cargo generate-lockfile"));
    }

    #[test]
    fn lockfile_problems_never_fail_the_run() {
        // Reproducibility is advisory: a project without a committed lockfile
        // still builds, so doctor must not exit non-zero over it.
        for ignored in [LockIgnored::Yes, LockIgnored::No, LockIgnored::Unknown] {
            assert_eq!(failure_count(&[classify_cargo_lock(false, ignored)]), 0);
            assert_eq!(failure_count(&[classify_cargo_lock(true, ignored)]), 0);
        }
    }

    #[test]
    fn lockfile_check_skipped_outside_a_cargo_project() {
        let dir = tempfile::tempdir().unwrap();
        assert!(cargo_lock_check(dir.path()).is_none());
    }

    #[test]
    fn lockfile_check_warns_when_the_project_has_no_lockfile() {
        // The acceptance case, through the real wrapper: a cargo project with
        // no Cargo.lock warns whatever git says about the path.
        let dir = project_with_manifest("[package]\nname = \"x\"\nversion = \"0.1.0\"\n");
        let check = cargo_lock_check(dir.path()).unwrap();
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("not found"));
    }

    #[test]
    fn lockfile_check_reads_gitignore_from_the_project_repository() {
        let Some(dir) = git_project("[package]\nname = \"x\"\nversion = \"0.1.0\"\n") else {
            return; // git unavailable — nothing to probe
        };
        std::fs::write(dir.path().join("Cargo.lock"), "version = 4\n").unwrap();

        // Present, with no rule excluding it: version control will carry it.
        assert_eq!(cargo_lock_check(dir.path()).unwrap().status, Status::Pass);

        // The same lockfile, now excluded — CI would never see it.
        std::fs::write(dir.path().join(".gitignore"), "Cargo.lock\n").unwrap();
        let check = cargo_lock_check(dir.path()).unwrap();
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("excluded by .gitignore"));
    }

    #[test]
    fn lockfile_is_not_auto_fixable() {
        // `cargo generate-lockfile` is safe to run, but committing the result
        // is the user's call — so this is reported, never auto-fixed.
        assert!(remedy(&fail("Cargo.lock")).is_none());
    fn disk_space_reports_warn_when_below_threshold() {
        let check = classify_disk_space(Some(500 * 1024 * 1024), MIN_FREE_DISK_SPACE_BYTES);
        assert_eq!(check.status, Status::Warn);
        assert_eq!(check.name, "disk space");
        assert!(check.detail.contains("500.0 MB free (low;"));
        assert!(check.fix.unwrap().contains("free up disk space"));
    }

    #[test]
    fn disk_space_reports_warn_at_zero() {
        let check = classify_disk_space(Some(0), MIN_FREE_DISK_SPACE_BYTES);
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("0 B free (low;"));
    }

    #[test]
    fn disk_space_warns_when_available_is_none() {
        let check = classify_disk_space(None, MIN_FREE_DISK_SPACE_BYTES);
        assert_eq!(check.status, Status::Warn);
        assert_eq!(check.name, "disk space");
        assert!(check.detail.contains("could not determine"));
        assert!(check.fix.is_some());
    }

    #[test]
    fn disk_space_live_check_runs() {
        let check = disk_space_check(Path::new("."));
        assert_eq!(check.name, "disk space");
        assert!(matches!(check.status, Status::Pass | Status::Warn));
    }

    // ---- auto-fix (`--fix`) ----

    fn fail(name: &'static str) -> Check {
        Check {
            name,
            status: Status::Fail,
            detail: String::new(),
            fix: Some("x"),
        }
    }

    #[test]
    fn remedy_for_missing_target() {
        let r = remedy(&fail("wasm32v1-none target"));
        if rustup_available() {
            let r = r.unwrap();
            assert_eq!(r.check, "wasm32v1-none target");
            assert_eq!(r.program, "rustup");
            assert_eq!(r.command_line(), "rustup target add wasm32v1-none");
        } else {
            assert!(r.is_none());
        }
    }

    #[test]
    fn remedy_for_missing_stellar_cli() {
        let r = remedy(&fail("stellar-cli")).unwrap();
        assert_eq!(r.check, "stellar-cli");
        assert_eq!(r.program, "cargo");
        assert_eq!(r.command_line(), "cargo install --locked stellar-cli");
    }

    #[test]
    fn no_remedy_for_passing_or_warning_checks() {
        let pass = Check {
            name: "stellar-cli",
            status: Status::Pass,
            detail: String::new(),
            fix: None,
        };
        assert!(remedy(&pass).is_none());

        // Target check is only a Warn (not Fail) when rustup is missing, and
        // we cannot auto-install rustup — so no remedy.
        let warn = Check {
            name: "wasm32v1-none target",
            status: Status::Warn,
            detail: "rustup not found".into(),
            fix: Some("install rustup ..."),
        };
        assert!(remedy(&warn).is_none());
    }

    #[test]
    fn no_remedy_for_non_autofixable_checks() {
        for name in ["rustc", "cargo", "git", "soroban-sdk"] {
            assert!(
                remedy(&fail(name)).is_none(),
                "{name} should not be auto-fixable"
            );
        }
    }

    #[test]
    fn fixable_remedies_selects_only_autofixable_failures() {
        let checks = vec![
            fail("rustc"),
            fail("wasm32v1-none target"),
            fail("stellar-cli"),
            Check {
                name: "git",
                status: Status::Warn,
                detail: String::new(),
                fix: Some("x"),
            },
        ];
        let remedies = fixable_remedies(&checks);
        if rustup_available() {
            assert_eq!(remedies.len(), 2);
            assert_eq!(remedies[0].check, "wasm32v1-none target");
            assert_eq!(remedies[1].check, "stellar-cli");
        } else {
            assert_eq!(remedies.len(), 1);
            assert_eq!(remedies[0].check, "stellar-cli");
        }
    }

    #[test]
    fn no_remedies_when_nothing_is_fixable() {
        let checks = vec![fail("rustc"), fail("cargo")];
        assert!(fixable_remedies(&checks).is_empty());
    }

    #[test]
    fn fix_plan_lists_each_command() {
        let remedies = fixable_remedies(&[fail("wasm32v1-none target"), fail("stellar-cli")]);
        let plan = format_fix_plan(&remedies);
        if rustup_available() {
            assert!(plan.contains("rustup target add wasm32v1-none"));
        } else {
            assert!(!plan.contains("rustup target add wasm32v1-none"));
        }
        assert!(plan.contains("cargo install --locked stellar-cli"));
    }

    #[test]
    fn fix_summary_marks_success_and_failure() {
        let outcomes = vec![
            FixOutcome {
                check: "wasm32v1-none target",
                command: "rustup target add wasm32v1-none".into(),
                succeeded: true,
                detail: None,
            },
            FixOutcome {
                check: "stellar-cli",
                command: "cargo install --locked stellar-cli".into(),
                succeeded: false,
                detail: Some("exited with status 101".into()),
            },
        ];
        let summary = format_fix_summary(&outcomes);
        assert!(summary.contains("✓ rustup target add wasm32v1-none"));
        assert!(summary.contains("✗ cargo install --locked stellar-cli"));
        assert!(summary.contains("exited with status 101"));
    }
}
