//! Minimum toolchain versions required to build a scaffolded Soroban project,
//! and the pure helpers for probing installed tool versions.
//!
//! Shared between `doctor` (which checks these on the host), `optimize`
//! (which pre-flights `stellar-cli` before shelling out to it) and
//! `scaffold` (which pins the same versions into a generated
//! `.devcontainer/`), so the modules can never drift apart.

/// Minimum Rust version able to target [`WASM_TARGET`].
pub const MIN_RUST: (u32, u32) = (1, 84);

/// Minimum `stellar-cli` version.
pub const MIN_STELLAR: (u32, u32) = (21, 0);

/// The wasm target Soroban contracts compile to.
pub const WASM_TARGET: &str = "wasm32v1-none";

/// Parse `X.Y` out of a `tool X.Y.Z ...` version line and compare against a
/// minimum. Unparseable versions count as too old.
///
/// Deliberately lenient: version lines vary by tool (`rustc 1.84.0 (...)`,
/// `stellar-cli 27.0.1`, `docker version 27.3.1`) so we scan whitespace-
/// separated words for the first one that starts with `<digits>.<digits>`.
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
///
/// Used by `doctor` to compare a project's declared `soroban-sdk` version
/// against the pinned one, and to parse the version word out of a
/// `<tool> X.Y.Z` line.
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

/// Parse the version out of a `<tool> X.Y.Z ...` line, e.g.
/// `stellar-cli 27.0.1` -> `Some((27, 0, 1))`.
pub fn parse_tool_version(line: &str) -> Option<(u32, u32, u32)> {
    line.split_whitespace().find_map(parse_semverish)
}

/// Whether `line` names a `stellar-cli` release known to break forge
/// workflows, and if so, the version to upgrade to.
///
/// Denylist of releases that shipped a regression affecting forge's usage of
/// `stellar contract ...`.  Keep this small and justified — a spurious entry
/// blocks users on a working CLI.
pub fn known_broken_stellar_cli_replacement(line: &str) -> Option<&'static str> {
    let version = parse_tool_version(line)?;
    match version {
        (27, 0, 0) => Some("27.0.1"),
        (27, 0, 1) => Some("27.1.0"),
        (28, 0, 0) => Some("28.0.1"),
        (29, 0, 0) => Some("29.0.1"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_at_least_parses_common_shapes() {
        assert!(version_at_least("rustc 1.84.0 (abc 2025-01-01)", (1, 84)));
        assert!(version_at_least("rustc 1.90.1-nightly", (1, 84)));
        assert!(version_at_least("cargo 2.0.0", (1, 84)));
        assert!(version_at_least("stellar-cli 21.0.0", (21, 0)));
        assert!(!version_at_least("rustc 1.83.0", (1, 84)));
        assert!(!version_at_least("stellar-cli 20.3.0", (21, 0)));
        assert!(!version_at_least("garbage", (1, 84)));
    }

    #[test]
    fn parse_semverish_handles_cargo_requirements() {
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
    fn parse_tool_version_finds_the_version_word() {
        assert_eq!(parse_tool_version("stellar-cli 27.0.1"), Some((27, 0, 1)));
        assert_eq!(
            parse_tool_version("stellar 21.0.0 (abc 2025-01-01)"),
            Some((21, 0, 0))
        );
        assert_eq!(parse_tool_version("no version here"), None);
    }

    #[test]
    fn known_broken_stellar_cli_versions_are_mapped() {
        assert_eq!(
            known_broken_stellar_cli_replacement("stellar-cli 27.0.0"),
            Some("27.0.1")
        );
        assert_eq!(
            known_broken_stellar_cli_replacement("stellar-cli 27.0.1"),
            Some("27.1.0")
        );
        assert_eq!(
            known_broken_stellar_cli_replacement("stellar-cli 28.0.0"),
            Some("28.0.1")
        );
        assert_eq!(
            known_broken_stellar_cli_replacement("stellar-cli 29.0.0"),
            Some("29.0.1")
        );
        assert_eq!(
            known_broken_stellar_cli_replacement("stellar-cli 26.1.0"),
            None
        );
        assert_eq!(
            known_broken_stellar_cli_replacement("stellar-cli 30.0.0"),
            None
        );
        assert_eq!(known_broken_stellar_cli_replacement("garbage"), None);
    }
}
