//! The `soroban-forge config` subcommand: print the resolved configuration
//! and warn about unrecognized keys in `forge.toml`.

use std::path::Path;

use clap::{ArgMatches, Command};

use crate::config::{resolved_report, unknown_keys, ForgeConfig, CONFIG_FILE_NAME};
use crate::error::{ForgeError, Result};
use crate::plugin::{ForgeContext, ForgePlugin};

/// The `config` subcommand.
pub struct ConfigPlugin;

impl ForgePlugin for ConfigPlugin {
    fn name(&self) -> &'static str {
        "config"
    }

    fn command(&self) -> Command {
        Command::new("config")
            .about("Print the resolved project and user configuration (defaults filled in) and warn about unknown keys")
    }

    fn run(&self, _matches: &ArgMatches, ctx: &ForgeContext) -> Result<()> {
        // Re-read the raw file for unknown-key detection; the typed config in
        // `ctx` has already dropped anything it did not recognize.
        let path = ctx.project_config_path.as_deref();
        let strays = project_unknown_keys(path)?;
        let user_path = ForgeConfig::user_config_path();
        let user_strays = if let Some(user_path) = user_path.as_ref().filter(|p| p.is_file()) {
            let raw = std::fs::read_to_string(user_path)
                .map_err(ForgeError::io(format!("reading {}", user_path.display())))?;
            unknown_keys(&raw).map_err(|e| ForgeError::Config {
                path: user_path.clone(),
                message: e.to_string(),
            })?
        } else {
            Vec::new()
        };

        if ctx.json {
            let report = serde_json::json!({
                "config_file_present": path.is_some(),
                "user_config_file": user_path.as_ref().filter(|p| p.is_file()).map(|p| p.display().to_string()),
                "resolved": {
                    "project": {
                        "name": ctx.config.as_ref().and_then(|c| c.project.name.clone()),
                        "authors": ctx.config.as_ref().map(|c| c.project.authors.clone()).unwrap_or_default(),
                    },
                    "scaffold": {
                        "default_template": ctx.config.as_ref()
                            .and_then(|c| c.scaffold.default_template.clone())
                            .unwrap_or_else(|| "hello-world".to_string()),
                    },
                    "network": {
                        "name": ctx.config.as_ref().and_then(|c| c.network.name.clone()),
                        "rpc_url": ctx.config.as_ref().and_then(|c| c.network.rpc_url.clone()),
                        "passphrase": ctx.config.as_ref().and_then(|c| c.network.passphrase.clone()),
                    },
                    "identity": {
                        "default": ctx.config.as_ref().and_then(|c| c.identity.default.clone()),
                    },
                },
                "unknown_keys": strays,
                "user_unknown_keys": user_strays,
            });
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
            return Ok(());
        }

        if !ctx.quiet {
            if path.is_none() && !user_path.as_ref().is_some_and(|p| p.is_file()) {
                println!("# no project or user config found — showing defaults\n");
            } else if path.is_none() {
                println!("# no {CONFIG_FILE_NAME} found — showing user defaults\n");
            }
            print!("{}", resolved_report(&ctx.config));
        }
        // Warnings are diagnostics, not informational output: stderr, even
        // under --quiet (matching "errors still go to stderr").
        for key in &strays {
            if let Some(path) = path {
                eprintln!("warning: unknown key `{key}` in {}", path.display());
            }
        }
        if let Some(user_path) = user_path {
            for key in &user_strays {
                eprintln!("warning: unknown key `{key}` in {}", user_path.display());
            }
        }
        Ok(())
    }
}

fn project_unknown_keys(path: Option<&Path>) -> Result<Vec<String>> {
    let Some(path) = path else {
        return Ok(Vec::new());
    };
    let raw = std::fs::read_to_string(path)
        .map_err(ForgeError::io(format!("reading {}", path.display())))?;
    unknown_keys(&raw).map_err(|e| ForgeError::Config {
        path: path.to_path_buf(),
        message: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_in(dir: &std::path::Path, quiet: bool, json: bool) -> ForgeContext {
        ForgeContext::with_output(dir.to_path_buf(), 0, quiet, json, false).unwrap()
    }

    #[test]
    fn runs_without_a_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = ConfigPlugin;
        let matches = plugin.command().get_matches_from(["config"]);
        plugin
            .run(&matches, &ctx_in(dir.path(), false, false))
            .unwrap();
    }

    #[test]
    fn runs_with_partial_config_and_typos() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(CONFIG_FILE_NAME),
            "[project]\nname = \"demo\"\nnmae = \"oops\"\n",
        )
        .unwrap();
        let plugin = ConfigPlugin;
        let matches = plugin.command().get_matches_from(["config"]);
        plugin
            .run(&matches, &ctx_in(dir.path(), false, false))
            .unwrap();
    }

    #[test]
    fn finds_config_and_unknown_keys_in_parent_directory() {
        let project = tempfile::tempdir().unwrap();
        let subdirectory = project.path().join("contracts");
        std::fs::create_dir(&subdirectory).unwrap();
        let config_path = project.path().join(CONFIG_FILE_NAME);
        std::fs::write(
            &config_path,
            "[project]\nname = \"ancestor-project\"\nnmae = \"typo\"\n",
        )
        .unwrap();

        let ctx = ctx_in(&subdirectory, false, false);
        assert_eq!(
            ctx.project_config_path.as_deref(),
            Some(config_path.as_path())
        );
        assert_eq!(
            ctx.config.as_ref().and_then(|config| config.project.name.as_deref()),
            Some("ancestor-project")
        );
        assert_eq!(
            project_unknown_keys(ctx.project_config_path.as_deref()).unwrap(),
            vec![String::from("nmae")]
        );

        let plugin = ConfigPlugin;
        let matches = plugin.command().get_matches_from(["config"]);
        plugin.run(&matches, &ctx).unwrap();
    }

    #[test]
    fn name_matches_command() {
        let plugin = ConfigPlugin;
        assert_eq!(plugin.name(), plugin.command().get_name());
    }

    #[test]
    fn quiet_json_paths_run_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(CONFIG_FILE_NAME), "[scafold]\n").unwrap();
        let plugin = ConfigPlugin;
        let matches = plugin.command().get_matches_from(["config"]);
        plugin
            .run(&matches, &ctx_in(dir.path(), true, false))
            .unwrap();
        let matches = plugin.command().get_matches_from(["config"]);
        plugin
            .run(&matches, &ctx_in(dir.path(), false, true))
            .unwrap();
    }
}
