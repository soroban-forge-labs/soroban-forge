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
    /// Whether `--verbose` was passed.
    pub verbose: bool,
    /// Whether informational command output should be suppressed.
    pub quiet: bool,
}

impl ForgeContext {
    /// Build a context for `cwd`, loading `forge.toml` if present.
    pub fn new(cwd: PathBuf, verbose: bool) -> Result<Self> {
        Self::with_output(cwd, verbose, false)
    }

    /// Build a context with explicit output controls.
    pub fn with_output(cwd: PathBuf, verbose: bool, quiet: bool) -> Result<Self> {
        let config = ForgeConfig::load_from(&cwd)?;
        Ok(Self {
            cwd,
            config,
            verbose,
            quiet,
        })
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

    /// Execute the subcommand.
    fn run(&self, matches: &clap::ArgMatches, ctx: &ForgeContext) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_is_not_quiet_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ForgeContext::new(dir.path().to_path_buf(), false).unwrap();
        assert!(!ctx.quiet);
    }

    #[test]
    fn context_accepts_explicit_quiet_mode() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ForgeContext::with_output(dir.path().to_path_buf(), false, true).unwrap();
        assert!(ctx.quiet);
    }
}
