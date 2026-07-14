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
    /// Directory the CLI was invoked from.
    pub cwd: PathBuf,
    /// Parsed `forge.toml` from `cwd`, when present.
    pub config: Option<ForgeConfig>,
    /// Whether `--verbose` was passed.
    pub verbose: bool,
}

impl ForgeContext {
    /// Build a context for `cwd`, loading `forge.toml` if present.
    pub fn new(cwd: PathBuf, verbose: bool) -> Result<Self> {
        let config = ForgeConfig::load_from(&cwd)?;
        Ok(Self {
            cwd,
            config,
            verbose,
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
