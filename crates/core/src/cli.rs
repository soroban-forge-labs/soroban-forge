//! Command construction and dispatch.

use clap::{Arg, ArgAction, ArgMatches, Command};

use crate::error::{ForgeError, Result};
use crate::plugin::{ForgeContext, ForgePlugin};

/// Build the top-level `soroban-forge` command from the registered plugins.
pub fn build_command(plugins: &[Box<dyn ForgePlugin>]) -> Command {
    let mut cmd = Command::new("soroban-forge")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Scaffolding, test-harness and CI toolkit for Soroban smart contracts on Stellar")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .arg(
            Arg::new("verbose")
                .long("verbose")
                .short('v')
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Enable verbose (debug) logging"),
        );
    for plugin in plugins {
        cmd = cmd.subcommand(plugin.command());
    }
    cmd
}

/// Route parsed matches to the owning plugin.
pub fn dispatch(plugins: &[Box<dyn ForgePlugin>], matches: &ArgMatches) -> Result<()> {
    let verbose = matches.get_flag("verbose");
    let (name, sub_matches) = matches
        .subcommand()
        .ok_or_else(|| ForgeError::InvalidArgument("a subcommand is required".into()))?;

    let plugin = plugins
        .iter()
        .find(|p| p.name() == name)
        .ok_or_else(|| ForgeError::InvalidArgument(format!("unknown subcommand `{name}`")))?;

    let cwd = std::env::current_dir().map_err(ForgeError::io("determining current directory"))?;
    let ctx = ForgeContext::new(cwd, verbose)?;

    log::debug!("dispatching to plugin `{}`", plugin.name());
    plugin.run(sub_matches, &ctx)
}

/// Entry point used by the `soroban-forge` binary: parse `std::env::args`,
/// initialise logging and dispatch. clap handles `--help`/`--version`/usage
/// errors itself (printing and exiting), as users expect.
pub fn run(plugins: Vec<Box<dyn ForgePlugin>>) -> Result<()> {
    let matches = build_command(&plugins).get_matches();

    let level = if matches.get_flag("verbose") {
        "debug"
    } else {
        "info"
    };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(level))
        .format_timestamp(None)
        .try_init()
        .ok();

    dispatch(&plugins, &matches)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    struct DummyPlugin {
        ran: Arc<AtomicBool>,
    }

    impl ForgePlugin for DummyPlugin {
        fn name(&self) -> &'static str {
            "dummy"
        }

        fn command(&self) -> Command {
            Command::new("dummy")
                .about("test plugin")
                .arg(Arg::new("flag").long("flag").action(ArgAction::SetTrue))
        }

        fn run(&self, matches: &ArgMatches, _ctx: &ForgeContext) -> Result<()> {
            assert!(matches.get_flag("flag"));
            self.ran.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    fn dummy() -> (Vec<Box<dyn ForgePlugin>>, Arc<AtomicBool>) {
        let ran = Arc::new(AtomicBool::new(false));
        let plugins: Vec<Box<dyn ForgePlugin>> = vec![Box::new(DummyPlugin { ran: ran.clone() })];
        (plugins, ran)
    }

    #[test]
    fn help_lists_plugin_subcommands() {
        let (plugins, _) = dummy();
        let help = build_command(&plugins).render_long_help().to_string();
        assert!(help.contains("dummy"));
        assert!(help.contains("test plugin"));
    }

    #[test]
    fn version_is_workspace_version() {
        let (plugins, _) = dummy();
        let cmd = build_command(&plugins);
        assert_eq!(cmd.get_version(), Some(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn dispatch_routes_to_plugin() {
        let (plugins, ran) = dummy();
        let matches = build_command(&plugins)
            .try_get_matches_from(["soroban-forge", "dummy", "--flag"])
            .unwrap();
        dispatch(&plugins, &matches).unwrap();
        assert!(ran.load(Ordering::SeqCst));
    }

    #[test]
    fn unknown_subcommand_is_a_parse_error() {
        let (plugins, _) = dummy();
        let result = build_command(&plugins).try_get_matches_from(["soroban-forge", "nonexistent"]);
        assert!(result.is_err());
    }
}
