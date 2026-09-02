//! The plugin interface implemented by every soroban-forge feature module.
//!
//! Each module (scaffold, testgen, ci-presets, doctor) exposes exactly one
//! subcommand by implementing [`ForgePlugin`]. The core knows nothing about
//! the modules beyond this trait, which is what keeps the five modules
//! independently ownable.

use std::path::PathBuf;

use crate::config::ForgeConfig;
use crate::error::Result;

/// Everything a plugin gets to see about the invocation environment.
pub struct ForgeContext {
    /// Directory the CLI was invoked from. // cwd provided by caller
    pub cwd: PathBuf,
    /// Parsed `forge.toml` from `cwd`, when present.
    pub config: Option<ForgeConfig>,
    /// Number of times `-v`/`--verbose` was passed (0 = default, 1 = debug, 2+ = trace).
    pub verbose: u8,
    /// Whether informational command output should be suppressed.
    pub quiet: bool,
    /// Whether structured JSON should be printed instead of text output.
    pub json: bool,
    /// Whether interactive confirmations should be auto-accepted (`--yes`).
    pub yes: bool,
    /// Whether all network-capable operations are disabled (`--offline`).
    pub offline: bool,
    /// Explicit log level override, if provided.
    pub log_level: Option<String>,
    /// Global timeout override for network-capable operations, if provided.
    pub timeout_secs: Option<u64>,
}

impl ForgeContext {
    /// Build a context for `cwd`, loading `forge.toml` if present.
    pub fn new(cwd: PathBuf, verbose: u8) -> Result<Self> {
        Self::with_output(cwd, verbose, false, false, false)
    }

    /// Build a context with explicit output controls.
    pub fn with_output(
        cwd: PathBuf,
        verbose: u8,
        quiet: bool,
        json: bool,
        yes: bool,
    ) -> Result<Self> {
        Self::with_options(cwd, verbose, quiet, json, yes, false, None, None, None)
    }

    /// Build a context with all global invocation controls.
    ///
    /// `config_path`, when set (from `--config`), is loaded directly and
    /// errors if missing/invalid. Otherwise `forge.toml` is discovered by
    /// walking up from `cwd`, same as before.
    pub fn with_options(
        cwd: PathBuf,
        verbose: u8,
        quiet: bool,
        json: bool,
        yes: bool,
        offline: bool,
        log_level: Option<String>,
        timeout_secs: Option<u64>,
        config_path: Option<PathBuf>,
    ) -> Result<Self> {
        let config = match &config_path {
            Some(path) => Some(ForgeConfig::load_from_path(path)?),
            None => ForgeConfig::load_from(&cwd)?,
        };
        Ok(Self {
            cwd,
            config,
            verbose,
            quiet,
            json,
            yes,
            offline,
            log_level,
            timeout_secs,
        })
    }

    pub fn progress(&self, message: &str) {
        if !self.quiet && !self.json {
            eprintln!("==> {message}");
        }
    }
}

/// A soroban-forge subcommand provider.
///
/// Contract for implementors:
/// - [`name`](ForgePlugin::name) must equal the name of the `clap::Command`
///   returned by [`command`](ForgePlugin::command); the core routes on it.
/// - `run` receives the `ArgMatches` of *its own* subcommand only.
pub trait ForgePlugin {
    /// Subcommand name, e.g. `"new"` or `"doctor"`.
    fn name(&self) -> &'static str;

    /// The clap definition of this subcommand.
    fn command(&self) -> clap::Command;

    /// Hook run immediately before subcommand execution.
    fn pre_run(&self, _matches: &clap::ArgMatches, _ctx: &ForgeContext) -> Result<()> {
        Ok(())
    }

    /// Execute the subcommand.
    fn run(&self, matches: &clap::ArgMatches, ctx: &ForgeContext) -> Result<()>;

    /// Hook run after subcommand execution completes.
    fn post_run(&self, _matches: &clap::ArgMatches, _ctx: &ForgeContext) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_is_not_quiet_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ForgeContext::new(dir.path().to_path_buf(), 0).unwrap();
        assert!(!ctx.quiet);
        assert!(!ctx.json);
    }

    #[test]
    fn context_accepts_explicit_quiet_mode() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ForgeContext::with_output(dir.path().to_path_buf(), 0, true, false, false).unwrap();
        assert!(ctx.quiet);
    }

    #[test]
    fn context_accepts_explicit_json_mode() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ForgeContext::with_output(dir.path().to_path_buf(), 0, false, true, false).unwrap();
        assert!(ctx.json);
    }

    #[test]
    fn context_is_not_yes_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ForgeContext::new(dir.path().to_path_buf(), 0).unwrap();
        assert!(!ctx.yes);
    }

    #[test]
    fn context_accepts_explicit_yes_mode() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ForgeContext::with_output(dir.path().to_path_buf(), 0, false, false, true).unwrap();
        assert!(ctx.yes);
    }

    #[test]
    fn context_loads_explicit_config_path() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("custom.toml");
        std::fs::write(&config_path, "[defaults]\ntimeout_secs = 42\n").unwrap();

        let ctx = ForgeContext::with_options(
            dir.path().to_path_buf(),
            0,
            false,
            false,
            false,
            false,
            None,
            None,
            Some(config_path),
        )
        .unwrap();

        assert_eq!(
            ctx.config.as_ref().and_then(|c| c.defaults.timeout_secs),
            Some(42)
        );
    }

    #[test]
    fn context_errors_on_invalid_explicit_config_path() {
        let dir = tempfile::tempdir().unwrap();
        let missing_path = dir.path().join("nonexistent.toml");

        let res = ForgeContext::with_options(
            dir.path().to_path_buf(),
            0,
            false,
            false,
            false,
            false,
            None,
            None,
            Some(missing_path),
        );

        assert!(res.is_err());
    }
}