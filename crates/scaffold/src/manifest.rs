//! `template.toml` — per-template metadata read from the template's own
//! directory instead of being hardcoded in Rust source.
//!
//! ```toml
//! description = "SEP-41 fungible token"
//!
//! [[variable]]
//! name = "token_symbol"
//! prompt = "Token symbol"
//! default = "MYT"
//!
//! [post_generate]
//! hints = ["run `cargo test` before deploying"]
//! ```
//!
//! A template without a `template.toml` still works: [`TemplateManifest::load`]
//! falls back to a manifest with no description, no extra variables, and no
//! hints, so migration can happen one template at a time.

use serde::Deserialize;

use soroban_forge_core::{ForgeError, Result};

/// One extra variable a template declares beyond the built-in
/// `project_name` / `crate_name` / `author` / `sdk_version`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TemplateVariable {
    /// Placeholder name used in `{{name}}` inside template files.
    pub name: String,
    /// Question shown when prompting interactively.
    pub prompt: String,
    /// Value used when not prompting (quiet/non-interactive) and no
    /// `--var name=value` override was given.
    pub default: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
struct PostGenerate {
    #[serde(default)]
    hints: Vec<String>,
}

/// Parsed `template.toml`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct TemplateManifest {
    /// One-line description, shown by `soroban-forge templates`.
    pub description: Option<String>,
    /// Extra variables this template prompts for beyond the built-ins.
    #[serde(default, rename = "variable")]
    pub variables: Vec<TemplateVariable>,
    #[serde(default)]
    post_generate: PostGenerate,
}

impl TemplateManifest {
    /// Parse a `template.toml`'s contents. Empty/missing manifests are
    /// represented by the caller as `TemplateManifest::default()`, not by
    /// calling this with empty input.
    pub fn parse(raw: &str, template_name: &str) -> Result<Self> {
        toml::from_str(raw).map_err(|e| {
            ForgeError::Config {
                path: format!("templates/{template_name}/template.toml").into(),
                message: e.to_string(),
            }
        })
    }

    /// Post-generation hints to print after `next steps`, in declared order.
    pub fn hints(&self) -> &[String] {
        &self.post_generate.hints
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_manifest_is_default() {
        let m = TemplateManifest::default();
        assert_eq!(m.description, None);
        assert!(m.variables.is_empty());
        assert!(m.hints().is_empty());
    }

    #[test]
    fn parses_description_and_variables() {
        let raw = r#"
description = "SEP-41 fungible token"

[[variable]]
name = "token_symbol"
prompt = "Token symbol"
default = "MYT"

[[variable]]
name = "token_decimals"
prompt = "Decimals"
default = "7"

[post_generate]
hints = ["deploy with --decimals 7", "run cargo test first"]
"#;
        let m = TemplateManifest::parse(raw, "token").unwrap();
        assert_eq!(m.description.as_deref(), Some("SEP-41 fungible token"));
        assert_eq!(m.variables.len(), 2);
        assert_eq!(m.variables[0].name, "token_symbol");
        assert_eq!(m.variables[0].default, "MYT");
        assert_eq!(
            m.hints(),
            &["deploy with --decimals 7".to_string(), "run cargo test first".to_string()]
        );
    }

    #[test]
    fn variables_and_hints_default_to_empty() {
        let m = TemplateManifest::parse(r#"description = "x""#, "x").unwrap();
        assert!(m.variables.is_empty());
        assert!(m.hints().is_empty());
    }

    #[test]
    fn invalid_toml_is_a_config_error() {
        let err = TemplateManifest::parse("not [valid", "token").unwrap_err();
        assert!(matches!(err, ForgeError::Config { .. }));
    }
}
