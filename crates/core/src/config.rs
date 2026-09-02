//! Loading of the optional `forge.toml` project configuration file.
//!
//! ```toml
//! [project]
//! name = "my-contract"
//! authors = ["Ada Lovelace <ada@example.com>"]
//!
//! [scaffold]
//! default_template = "hello-world"
//! ```

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{ForgeError, Result};

/// File name looked up in the working directory.
pub const CONFIG_FILE_NAME: &str = "forge.toml"; // project config filename

/// Parsed contents of `forge.toml`. All fields are optional so a partial
/// config (or no config at all) is always valid.
#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
pub struct ForgeConfig {
    #[serde(default)]
    pub project: ProjectConfig,
    #[serde(default)]
    pub scaffold: ScaffoldConfig,
    #[serde(default)]
    pub defaults: DefaultsConfig,
    #[serde(default)]
    pub network: NetworkConfig,
}

#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
pub struct ProjectConfig {
    pub name: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
pub struct ScaffoldConfig {
    /// Template used by `soroban-forge new` when `--template` is not given.
    pub default_template: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
pub struct DefaultsConfig {
    pub timeout_secs: Option<u64>,
    #[serde(default, rename = "ci-init", alias = "ci_init")]
    pub ci_init: CiInitDefaults,
}

#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
pub struct CiInitDefaults {
    pub max_size: Option<u64>,
}

#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
pub struct NetworkConfig {
    pub name: Option<String>,
    pub rpc_url: Option<String>,
}

impl ForgeConfig {
    /// Load `forge.toml` from `dir`, returning `Ok(None)` when the file does
    /// not exist and an error only when it exists but cannot be parsed.
    pub fn load_from(dir: &Path) -> Result<Option<Self>> {
        let Some(path) = find_config_path(dir) else {
            return Ok(None);
        };
        let raw = std::fs::read_to_string(&path)
            .map_err(ForgeError::io(format!("reading {}", path.display())))?;
        let config = toml::from_str(&raw).map_err(|e| ForgeError::Config {
            path: path.clone(),
            message: e.to_string(),
        })?;
        Ok(Some(config))
    }

    /// Load config from an explicit file path (`--config`), erroring if the
    /// file does not exist or fails to parse — unlike `load_from`, an
    /// explicitly-named path is not optional.
    pub fn load_from_path(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .map_err(ForgeError::io(format!("reading {}", path.display())))?;
        toml::from_str(&raw).map_err(|e| ForgeError::Config {
            path: path.to_path_buf(),
            message: e.to_string(),
        })
    }

    /// First configured author, if any.
    pub fn author(&self) -> Option<&str> {
        self.project.authors.first().map(String::as_str)
    }
}

fn find_config_path(dir: &Path) -> Option<PathBuf> {
    let mut current = Some(dir);
    while let Some(path) = current {
        let candidate = path.join(CONFIG_FILE_NAME);
        if candidate.is_file() {
            return Some(candidate);
        }
        current = path.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(ForgeConfig::load_from(dir.path()).unwrap(), None);
    }

    #[test]
    fn parses_full_config() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(CONFIG_FILE_NAME),
            r#"
[project]
name = "demo"
authors = ["Ada <ada@example.com>"]

[scaffold]
default_template = "token"

[defaults.ci-init]
max_size = 65536
"#,
        )
        .unwrap();

        let config = ForgeConfig::load_from(dir.path()).unwrap().unwrap();
        assert_eq!(config.project.name.as_deref(), Some("demo"));
        assert_eq!(config.author(), Some("Ada <ada@example.com>"));
        assert_eq!(config.scaffold.default_template.as_deref(), Some("token"));
        assert_eq!(config.defaults.ci_init.max_size, Some(65_536));
    }

    #[test]
    fn empty_file_is_valid() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(CONFIG_FILE_NAME), "").unwrap();
        let config = ForgeConfig::load_from(dir.path()).unwrap().unwrap();
        assert_eq!(config, ForgeConfig::default());
    }

    #[test]
    fn invalid_toml_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(CONFIG_FILE_NAME), "not [valid").unwrap();
        assert!(ForgeConfig::load_from(dir.path()).is_err());
    }

    #[test]
    fn load_from_path_errors_on_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.toml");
        assert!(ForgeConfig::load_from_path(&missing).is_err());
    }

    #[test]
    fn load_from_path_parses_explicit_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("custom.toml");
        std::fs::write(&path, "[defaults]\ntimeout_secs = 30\n").unwrap();
        let config = ForgeConfig::load_from_path(&path).unwrap();
        assert_eq!(config.defaults.timeout_secs, Some(30));
    }

    #[test]
    fn load_from_path_errors_on_invalid_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, "not [valid").unwrap();
        assert!(ForgeConfig::load_from_path(&path).is_err());
    }
}

/// Dotted paths of keys in `raw` that no [`ForgeConfig`] field matches.
///
/// The typed parse silently ignores unrecognized keys, so typos like
/// `default_templte` vanish without a trace. This walks the raw TOML tree
/// against the known-key schema and reports strays (e.g. `scafold`,
/// `project.nmae`) so `soroban-forge config` can warn about them.
pub fn unknown_keys(raw: &str) -> std::result::Result<Vec<String>, toml::de::Error> {
    let table: toml::Table = toml::from_str(raw)?;
    let mut strays = Vec::new();
    for (key, value) in &table {
        match key.as_str() {
            "project" => collect_strays(value, &["name", "authors"], "project", &mut strays),
            "scaffold" => {
                collect_strays(value, &["default_template"], "scaffold", &mut strays)
            }
            "defaults" => collect_strays(value, &["timeout_secs"], "defaults", &mut strays),
            "network" => collect_strays(value, &["name", "rpc_url"], "network", &mut strays),
            "defaults" => {
                collect_strays(value, &["timeout_secs", "ci-init", "ci_init"], "defaults", &mut strays)
                if let toml::Value::Table(table) = value {
                    if let Some(ci_init) = table.get("ci-init").or_else(|| table.get("ci_init")) {
                        collect_strays(ci_init, &["max_size"], "defaults.ci-init", &mut strays);
                    }
                }
            }
            _ => strays.push(key.clone()),
        }
    }
    Ok(strays)
}

/// Push dotted paths for any key of `value` (when it is a table) that is not
/// in `known`. Non-table values at section level are legal TOML shapes the
/// typed parse would already have rejected, so they are ignored here.
fn collect_strays(value: &toml::Value, known: &[&str], prefix: &str, out: &mut Vec<String>) {
    if let toml::Value::Table(table) = value {
        for key in table.keys() {
            if !known.contains(&key.as_str()) {
                out.push(format!("{prefix}.{key}"));
            }
        }
    }
}

/// Render the effective configuration as forge.toml-shaped text, with
/// defaults filled in for anything unset.
///
/// `config` is the loaded file (or `None` when no forge.toml exists) — the
/// output is identical in shape either way, which is the point: this shows
/// the configuration commands actually run with.
pub fn resolved_report(config: &Option<ForgeConfig>) -> String {
    let config = config.clone().unwrap_or_default();
    let mut out = String::new();

    out.push_str("[project]\n");
    match &config.project.name {
        Some(name) => out.push_str(&format!("name = \"{name}\"\n")),
        None => out.push_str("# name = (unset)\n"),
    }
    if config.project.authors.is_empty() {
        out.push_str("authors = []\n");
    } else {
        let quoted: Vec<String> = config
            .project
            .authors
            .iter()
            .map(|a| format!("\"{a}\""))
            .collect();
        out.push_str(&format!("authors = [{}]\n", quoted.join(", ")));
    }

    out.push_str("\n[scaffold]\n");
    match &config.scaffold.default_template {
        Some(t) => out.push_str(&format!("default_template = \"{t}\"\n")),
        // Keep in sync with DEFAULT_TEMPLATE in soroban-forge-scaffold; core
        // cannot depend on scaffold without a dependency cycle.
        None => out.push_str("default_template = \"hello-world\"  # default\n"),
    }
    out.push_str("\n[defaults]\n");
    match config.defaults.timeout_secs {
        Some(timeout_secs) => out.push_str(&format!("timeout_secs = {timeout_secs}\n")),
        None => out.push_str("# timeout_secs = (unset)\n"),
    }

    out.push_str("\n[network]\n");
    match &config.network.name {
        Some(name) => out.push_str(&format!("name = \"{name}\"\n")),
        None => out.push_str("# name = (unset)\n"),
    }
    match &config.network.rpc_url {
        Some(url) => out.push_str(&format!("rpc_url = \"{url}\"\n")),
        None => out.push_str("# rpc_url = (unset)\n"),
    out.push_str("\n[defaults.ci-init]\n");
    match config.defaults.ci_init.max_size {
        Some(max_size) => out.push_str(&format!("max_size = {max_size}\n")),
        None => out.push_str("# max_size = (unset)\n"),
    }
    out
}

#[cfg(test)]
mod resolved_tests {
    use super::*;

    #[test]
    fn no_config_prints_all_defaults() {
        let report = resolved_report(&None);
        assert!(report.contains("[project]"));
        assert!(report.contains("# name = (unset)"));
        assert!(report.contains("authors = []"));
        assert!(report.contains("default_template = \"hello-world\""));
        assert!(report.contains("[defaults.ci-init]"));
        assert!(report.contains("# max_size = (unset)"));
    }

    #[test]
    fn partial_config_merges_with_defaults() {
        let config: ForgeConfig =
            toml::from_str("[project]\nname = \"demo\"\n").unwrap();
        let report = resolved_report(&Some(config));
        assert!(report.contains("name = \"demo\""));
        assert!(report.contains("authors = []"));
        assert!(report.contains("default_template = \"hello-world\""));
    }

    #[test]
    fn full_config_shows_configured_values() {
        let config: ForgeConfig = toml::from_str(
            "[project]\nname = \"demo\"\nauthors = [\"Ada\"]\n[scaffold]\ndefault_template = \"token\"\n",
        )
        .unwrap();
        let report = resolved_report(&Some(config));
        assert!(report.contains("authors = [\"Ada\"]"));
        assert!(report.contains("default_template = \"token\"\n"));
        assert!(!report.contains("# default"));
    }

    #[test]
    fn detects_unknown_top_level_section() {
        let strays = unknown_keys("[scafold]\ndefault_template = \"token\"\n").unwrap();
        assert_eq!(strays, vec!["scafold"]);
    }

    #[test]
    fn detects_unknown_nested_keys() {
        let strays = unknown_keys(
            "[project]\nnmae = \"demo\"\n[scaffold]\ndefault_templte = \"token\"\n",
        )
        .unwrap();
        assert_eq!(strays, vec!["project.nmae", "scaffold.default_templte"]);
    }

    #[test]
    fn valid_keys_produce_no_warnings() {
        let strays = unknown_keys(
            "[project]\nname = \"demo\"\nauthors = []\n[scaffold]\ndefault_template = \"token\"\n",
        )
        .unwrap();
        assert!(strays.is_empty());
    }

    #[test]
    fn empty_file_produces_no_warnings() {
        assert!(unknown_keys("").unwrap().is_empty());
    }
}