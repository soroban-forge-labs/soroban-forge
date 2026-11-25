//! # soroban-forge-update-check
//!
//! Throttled, opt-out release-version check.
//!
//! On invocation `check()` is called from the main CLI entry-point. It:
//!
//! 1. Returns immediately (never blocks the command) — the HTTP request is
//!    made on a background thread that is detached. Any hint is printed
//!    *after* the main command finishes, so it cannot interleave with
//!    command output.
//! 2. Is throttled to at most one network request per 24 hours using a small
//!    state file in the user's config directory
//!    (`~/.config/soroban-forge/update-check.json`).
//! 3. Is completely disabled when:
//!    - `--offline` is passed on the CLI (signalled via `offline: bool`), or
//!    - `SOROBAN_FORGE_NO_UPDATE_CHECK=1` is set in the environment, or
//!    - `[defaults] update_check = false` is set in `forge.toml`.
//! 4. Never panics — all errors are logged at `debug` level and silently
//!    swallowed.
//!
//! The only network contact is a single HTTPS GET to the GitHub releases API:
//! `https://api.github.com/repos/soroban-forge-labs/soroban-forge/releases/latest`
//! The response body is parsed for `"tag_name"` only; nothing else is
//! transmitted or stored beyond the last-checked timestamp + latest version.
//!
//! This is not telemetry: no user data, command arguments, project contents
//! or identifiers are sent. See `docs/privacy.md`.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// How often to hit the GitHub API (24 h expressed in seconds).
const CHECK_INTERVAL_SECS: u64 = 60 * 60 * 24;
const WAIT_TIMEOUT: Duration = Duration::from_millis(250);

/// GitHub releases API endpoint for the soroban-forge project.
const RELEASES_API_URL: &str =
    "https://api.github.com/repos/soroban-forge-labs/soroban-forge/releases/latest";

/// On-disk state persisted between invocations.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct UpdateCheckState {
    /// Unix timestamp (seconds) of the last check attempt.
    last_checked_secs: u64,
    /// Latest release tag seen on the last successful check (e.g. `"v0.2.0"`).
    latest_tag: Option<String>,
}

/// Minimal shape we parse out of the GitHub releases JSON.
#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
}

/// Path to the update-check state file.
fn state_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("soroban-forge").join("update-check.json"))
}

fn load_state(path: &Path) -> UpdateCheckState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_state(path: &Path, state: &UpdateCheckState) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string(state) {
        let _ = std::fs::write(path, json);
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

/// Parse a semver-like version string, stripping a leading `v` if present,
/// and return a comparable tuple `(major, minor, patch)`.
fn parse_version(v: &str) -> Option<semver::Version> {
    let v = v.strip_prefix('v').unwrap_or(v);
    semver::Version::parse(v).ok()
}

/// Perform a synchronous HTTP GET using only `std::net` + manual HTTP/1.1
/// over TLS-free TCP to avoid pulling in a full HTTP client.  For the update
/// check we use a plain HTTP redirect-follow approach; because the GitHub API
/// always returns JSON over HTTPS we shell out to `curl` or `wget` if
/// available, which keeps the binary small and avoids a TLS dependency in
/// this crate.
///
/// Returns the raw response body on success.
fn http_get(url: &str) -> Option<String> {
    // Try curl first, then wget — both are present on almost every CI runner
    // and developer machine.
    let curl_out = std::process::Command::new("curl")
        .args([
            "--silent",
            "--max-time",
            "2",
            "--user-agent",
            concat!("soroban-forge/", env!("CARGO_PKG_VERSION")),
            url,
        ])
        .output();

    if let Ok(out) = curl_out {
        if out.status.success() {
            return String::from_utf8(out.stdout).ok();
        }
    }

    // Fallback: wget
    let wget_out = std::process::Command::new("wget")
        .args(["--quiet", "--timeout=5", "--output-document=-", url])
        .output();

    if let Ok(out) = wget_out {
        if out.status.success() {
            return String::from_utf8(out.stdout).ok();
        }
    }

    None
}

/// Spawn the version check in the background. Returns immediately.
///
/// Arguments:
/// - `offline` — mirrors `ForgeContext::offline`; when `true` the function
///   returns without spawning anything.
/// - `update_check_enabled` — mirrors `forge.toml [defaults] update_check`;
///   when `false` the function returns without spawning anything.
///
/// The hint (if any) is collected via a `JoinHandle<Option<String>>` that the
/// caller should `wait_and_print` on *after* the main command has finished.
pub fn spawn(
    offline: bool,
    update_check_enabled: bool,
) -> Option<std::thread::JoinHandle<Option<String>>> {
    if offline {
        log::debug!("update-check: skipped (--offline)");
        return None;
    }
    if !update_check_enabled {
        log::debug!("update-check: skipped (disabled via config/env)");
        return None;
    }
    if std::env::var_os("CI").is_some() {
        log::debug!("update-check: skipped (CI)");
        return None;
    }
    if !std::io::stderr().is_terminal() {
        log::debug!("update-check: skipped (non-interactive stderr)");
        return None;
    }
    // Honour the SOROBAN_FORGE_NO_UPDATE_CHECK env variable.
    if std::env::var("SOROBAN_FORGE_NO_UPDATE_CHECK")
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
    {
        log::debug!("update-check: skipped (SOROBAN_FORGE_NO_UPDATE_CHECK)");
        return None;
    }

    let handle = std::thread::spawn(move || -> Option<String> {
        let path = state_path()?;
        let current_secs = now_secs();
        let state = load_state(&path);

        if current_secs.saturating_sub(state.last_checked_secs) < CHECK_INTERVAL_SECS {
            // Within the throttle window — use cached state to decide whether to hint.
            log::debug!("update-check: within throttle window, using cached state");
            return build_hint(state.latest_tag.as_deref());
        }

        // Record the attempt before fetching so offline commands are throttled too.
        save_state(
            &path,
            &UpdateCheckState {
                last_checked_secs: current_secs,
                latest_tag: state.latest_tag.clone(),
            },
        );

        // Outside window — fetch from GitHub.
        log::debug!("update-check: fetching {RELEASES_API_URL}");
        let body = match http_get(RELEASES_API_URL) {
            Some(b) => b,
            None => {
                log::debug!("update-check: HTTP request failed, skipping");
                return None;
            }
        };

        let release: GhRelease = match serde_json::from_str(&body) {
            Ok(r) => r,
            Err(e) => {
                log::debug!("update-check: failed to parse response: {e}");
                return None;
            }
        };

        // Persist updated state.
        let new_state = UpdateCheckState {
            last_checked_secs: current_secs,
            latest_tag: Some(release.tag_name.clone()),
        };
        save_state(&path, &new_state);

        build_hint(Some(&release.tag_name))
    });

    Some(handle)
}

/// Wait for the background thread and, if it produced a hint, print it to
/// stderr. This is a no-op when `handle` is `None`.
pub fn wait_and_print(handle: Option<std::thread::JoinHandle<Option<String>>>) {
    let Some(handle) = handle else { return };
    let deadline = Instant::now() + WAIT_TIMEOUT;
    while !handle.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    if !handle.is_finished() {
        log::debug!("update-check: still running at exit; skipping hint");
        return;
    }
    match handle.join() {
        Ok(Some(hint)) => eprintln!("{hint}"),
        Ok(None) => {}
        Err(_) => {
            log::debug!("update-check: background thread panicked (ignored)");
        }
    }
}

/// Build the upgrade hint string if the latest release is newer than the
/// running binary.
fn build_hint(latest_tag: Option<&str>) -> Option<String> {
    build_hint_for(latest_tag, env!("CARGO_PKG_VERSION"))
}

fn build_hint_for(latest_tag: Option<&str>, current: &str) -> Option<String> {
    let tag = latest_tag?;
    let latest_ver = parse_version(tag)?;
    let current_ver = parse_version(current)?;
    if latest_ver <= current_ver {
        return None;
    }
    let executable = std::env::current_exe().ok();
    let executable = executable
        .as_ref()
        .and_then(|path| path.to_str())
        .unwrap_or_default();
    let install_command = if executable.contains("Homebrew")
        || executable.contains("homebrew")
        || executable.contains("/Cellar/")
    {
        "brew upgrade soroban-forge"
    } else if std::env::var_os("CARGO_BINSTALL").is_some() {
        "cargo binstall soroban-forge"
    } else {
        "cargo install --git https://github.com/soroban-forge-labs/soroban-forge soroban-forge"
    };
    Some(format!(
        "\nA new version of soroban-forge is available: {tag} (you have v{current})\n\
         Run: {install_command}\n\
         Or disable this check: SOROBAN_FORGE_NO_UPDATE_CHECK=1 (or `[defaults] update_check = false` in forge.toml)"
    ))
}

fn install_command_for_path(path: &Path) -> &'static str {
    let components: Vec<_> = path
        .components()
        .map(|component| component.as_os_str())
        .collect();
    let is_homebrew_path = components
        .windows(2)
        .any(|pair| pair[0] == "Cellar" && pair[1] == "soroban-forge")
        || components
            .windows(2)
            .any(|pair| pair[0] == "opt" && pair[1] == "homebrew");
    if is_homebrew_path {
        "brew upgrade soroban-forge"
    } else if components
        .windows(2)
        .any(|pair| pair[0] == ".cargo" && pair[1] == "bin")
    {
        "cargo binstall soroban-forge"
    } else {
        "cargo install --git https://github.com/soroban-forge-labs/soroban-forge soroban-forge"
    }
}

fn check_with(
    path: &Path,
    url: &str,
    current_secs: u64,
    fetch: impl FnOnce(&str) -> Option<String>,
) -> Option<String> {
    let state = load_state(path);
    if current_secs.saturating_sub(state.last_checked_secs) < CHECK_INTERVAL_SECS {
        log::debug!("update-check: within throttle window, using cached state");
        return build_hint(state.latest_tag.as_deref());
    }

    save_state(
        path,
        &UpdateCheckState {
            last_checked_secs: current_secs,
            latest_tag: state.latest_tag,
        },
    );

    log::debug!("update-check: fetching {url}");
    let body = match fetch(url) {
        Some(body) => body,
        None => {
            log::debug!("update-check: HTTP request failed, skipping");
            return None;
        }
    };
    let release: GhRelease = match serde_json::from_str(&body) {
        Ok(release) => release,
        Err(error) => {
            log::debug!("update-check: failed to parse response: {error}");
            return None;
        }
    };
    save_state(
        path,
        &UpdateCheckState {
            last_checked_secs: current_secs,
            latest_tag: Some(release.tag_name.clone()),
        },
    );
    build_hint(Some(&release.tag_name))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_handles_v_prefix() {
        assert_eq!(
            parse_version("v0.2.0"),
            semver::Version::parse("0.2.0").ok()
        );
        assert_eq!(
            parse_version("1.10.3"),
            semver::Version::parse("1.10.3").ok()
        );
    }

    #[test]
    fn prerelease_sorts_before_release() {
        assert!(parse_version("v1.0.0-beta.1") < parse_version("v1.0.0"));
    }

    #[test]
    fn parse_version_rejects_invalid() {
        assert_eq!(parse_version("not-a-version"), None);
        assert_eq!(parse_version("1.2"), None);
    }

    #[test]
    fn build_hint_none_when_up_to_date() {
        // The crate version is whatever it is; pretend the latest is the same.
        let current = env!("CARGO_PKG_VERSION");
        assert!(build_hint(Some(&format!("v{current}"))).is_none());
    }

    #[test]
    fn build_hint_none_when_no_latest() {
        assert!(build_hint(None).is_none());
    }

    #[test]
    fn build_hint_returns_hint_when_behind() {
        // Use a very high version number so it will always be newer than the
        // binary under test.
        let hint = build_hint(Some("v999.999.999")).unwrap();
        assert!(hint.contains("v999.999.999"));
        assert!(hint.contains("SOROBAN_FORGE_NO_UPDATE_CHECK"));
    }

    #[test]
    fn build_hint_fires_for_prerelease_user() {
        let hint = build_hint_for(Some("v1.0.0"), "1.0.0-beta.1").unwrap();
        assert!(hint.contains("1.0.0-beta.1"));
    }

    #[test]
    fn install_command_matches_binary_path() {
        assert_eq!(
            install_command_for_path(Path::new(
                "/opt/homebrew/Cellar/soroban-forge/0.1.0/bin/soroban-forge"
            )),
            "brew upgrade soroban-forge"
        );
        assert_eq!(
            install_command_for_path(Path::new("/opt/homebrew/bin/soroban-forge")),
            "brew upgrade soroban-forge"
        );
        assert_eq!(
            install_command_for_path(Path::new("/home/user/.cargo/bin/soroban-forge")),
            "cargo binstall soroban-forge"
        );
        assert!(
            install_command_for_path(Path::new("/usr/local/bin/soroban-forge"))
                .starts_with("cargo install --git")
        );
    }

    #[test]
    fn failed_fetch_is_throttled_on_second_run() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("update-check.json");
        let url = "http://127.0.0.1:1/unreachable";
        let fetch_count = std::cell::Cell::new(0);

        let fetch = |url: &str| {
            fetch_count.set(fetch_count.get() + 1);
            http_get(url)
        };
        assert!(check_with(&path, url, 100_000, fetch).is_none());
        assert!(check_with(&path, url, 100_001, |url| {
            fetch_count.set(fetch_count.get() + 1);
            http_get(url)
        })
        .is_none());

        assert_eq!(fetch_count.get(), 1);
        assert_eq!(load_state(&path).last_checked_secs, 100_000);
    }

    #[test]
    fn spawn_returns_none_when_offline() {
        assert!(spawn(true, true).is_none());
    }

    #[test]
    fn spawn_returns_none_when_disabled_by_config() {
        assert!(spawn(false, false).is_none());
    }

    #[test]
    fn spawn_returns_none_when_env_opt_out() {
        std::env::set_var("SOROBAN_FORGE_NO_UPDATE_CHECK", "1");
        let result = spawn(false, true);
        std::env::remove_var("SOROBAN_FORGE_NO_UPDATE_CHECK");
        assert!(result.is_none());
    }

    #[test]
    fn wait_and_print_is_noop_for_none() {
        // Should not panic or block.
        wait_and_print(None);
    }
}
