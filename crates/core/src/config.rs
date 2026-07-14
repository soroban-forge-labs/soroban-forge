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

use std::path::Path;

use serde::Deserialize;

use crate::error::{ForgeError, Result};

/// File name looked up in the working directory.
pub const CONFIG_FILE_NAME: &str = "forge.toml";

/// Parsed contents of `forge.toml`. All fields are optional so a partial
/// config (or no config at all) is always valid.
#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
pub struct ForgeConfig {
    #[serde(default)]
    pub project: ProjectConfig,
    #[serde(default)]
    pub scaffold: ScaffoldConfig,
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

impl ForgeConfig {
    /// Load `forge.toml` from `dir`, returning `Ok(None)` when the file does
    /// not exist and an error only when it exists but cannot be parsed.
    pub fn load_from(dir: &Path) -> Result<Option<Self>> {
        let path = dir.join(CONFIG_FILE_NAME);
        if !path.is_file() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(&path)
            .map_err(ForgeError::io(format!("reading {}", path.display())))?;
        let config = toml::from_str(&raw).map_err(|e| ForgeError::Config {
            path: path.clone(),
            message: e.to_string(),
        })?;
        Ok(Some(config))
    }

    /// First configured author, if any.
    pub fn author(&self) -> Option<&str> {
        self.project.authors.first().map(String::as_str)
    }
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
"#,
        )
        .unwrap();

        let config = ForgeConfig::load_from(dir.path()).unwrap().unwrap();
        assert_eq!(config.project.name.as_deref(), Some("demo"));
        assert_eq!(config.author(), Some("Ada <ada@example.com>"));
        assert_eq!(config.scaffold.default_template.as_deref(), Some("token"));
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
}
