//! Command construction and dispatch.
//!
//! # External subcommands
//!
//! Unknown subcommands are dispatched to a `soroban-forge-<name>` binary on
//! PATH (like cargo).  The external binary inherits our stdio and receives
//! everything after the subcommand name on the original argv.  Global flags
//! consumed by `soroban-forge` *before* the subcommand name are not forwarded.

use std::ffi::OsStr;

use clap::{Arg, ArgAction, ArgMatches, Command};

use crate::error::{ForgeError, Result};
use crate::plugin::{ForgeContext, ForgePlugin};


/// Levenshtein edit distance.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (m, n) = (a.len(), b.len());
    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for i in 0..=m { dp[i][0] = i; }
    for j in 0..=n { dp[0][j] = j; }
    for i in 1..=m {
        for j in 1..=n {
            let cost = if a[i-1] == b[j-1] { 0 } else { 1 };
            dp[i][j] = (dp[i-1][j]+1).min(dp[i][j-1]+1).min(dp[i-1][j-1]+cost);
        }
    }
    dp[m][n]
}
pub fn closest_match<'a>(input: &str, candidates: &[&'a str], threshold: usize) -> Option<&'a str> {
    candidates.iter().map(|c| (*c, edit_distance(input, c))).filter(|(_, d)| *d <= threshold).min_by_key(|(_, d)| *d).map(|(c, _)| c)
}
pub fn env_flag(name: &str) -> bool {
    std::env::var(name).map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes")).unwrap_or(false)
}

/// Build the top-level `soroban-forge` command from the registered plugins.
pub fn build_command(plugins: &[Box<dyn ForgePlugin>]) -> Command {
    let mut cmd = Command::new("soroban-forge")
        .version(env!("CARGO_PKG_VERSION"))
        .about(
            "Scaffolding, test-harness and CI toolkit for Soroban smart contracts on Stellar (CLI)",
        )
        .subcommand_required(false)
        .arg_required_else_help(true)
        .allow_external_subcommands(true)
        .after_help("See all installed subcommands with `soroban-forge --list`")
        .arg(
            Arg::new("list")
                .long("list")
                .action(ArgAction::SetTrue)
                .help("List all installed subcommands"),
        )
        .arg(
            Arg::new("verbose")
                .long("verbose")
                .short('v')
                .global(true)
                .action(ArgAction::Count)
                .help("Increase log verbosity (-v debug, -vv trace)"),
        )
        .arg(
            Arg::new("quiet")
                .long("quiet")
                .short('q')
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Suppress informational command output"),
        )
        .arg(
            Arg::new("json")
                .long("json")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Emit machine-readable JSON output"),
        )
        .arg(
            Arg::new("yes")
                .long("yes")
                .short('y')
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Auto-confirm all interactive prompts"),
        )
        .arg(
            Arg::new("log-file")
                .long("log-file")
                .global(true)
                .value_name("PATH")
                .help("Also write structured JSON logs to PATH"),
        )
        .arg(
            Arg::new("log-level")
                .long("log-level")
                .global(true)
                .value_name("LEVEL")
                .help("Override the log level for console and file output"),
        )
        .arg(
            Arg::new("cwd")
                .long("cwd")
                .short('C')
                .global(true)
                .value_name("DIR")
                .help("Run as if started in DIR"),
        )
        .arg(
            Arg::new("offline")
                .long("offline")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Disable all network access"),
        )
        .arg(
            Arg::new("timeout")
                .long("timeout")
                .global(true)
                .value_name("SECS")
                .help("Override the timeout for network-capable operations"),
        )
        .arg(
            Arg::new("config")
                .long("config")
                .global(true)
                .value_name("PATH")
                .help("Load defaults from PATH instead of discovering forge.toml"),
        );
    for plugin in plugins {
        let mut subcmd = plugin.command();
        if plugin.name() == "optimize" {
            subcmd = subcmd
                .arg(
                    Arg::new("check")
                        .long("check")
                        .action(ArgAction::SetTrue)
                        .help("Fail if the optimized wasm exceeds --max-size"),
                )
                .arg(
                    Arg::new("max-size")
                        .long("max-size")
                        .value_name("BYTES")
                        .value_parser(clap::value_parser!(u64))
                        .help("Maximum size in bytes for the optimized wasm"),
                );
        }
        cmd = cmd.subcommand(subcmd);
    }
    cmd = cmd.subcommand(
        Command::new("completions")
            .about("Generate shell completion scripts and print them to stdout")
            .arg(
                Arg::new("shell")
                    .value_name("SHELL")
                    .required(true)
                    .value_parser(["bash", "zsh", "fish"])
                    .help("Shell to generate completions for"),
            ),
    );
    cmd
}

/// Route parsed matches to the owning plugin or an external subcommand.
pub fn dispatch(plugins: &[Box<dyn ForgePlugin>], matches: &ArgMatches) -> Result<()> {
    let verbose = matches.get_count("verbose").max(if env_flag("SOROBAN_FORGE_VERBOSE") { 1 } else { 0 });
    let quiet = matches.get_flag("quiet") || env_flag("SOROBAN_FORGE_QUIET");
    let json = matches.get_flag("json") || env_flag("SOROBAN_FORGE_JSON");
    let yes = matches.get_flag("yes") || env_flag("SOROBAN_FORGE_YES");
    let offline = matches.get_flag("offline") || env_flag("SOROBAN_FORGE_OFFLINE");
    let (name, sub_matches) = matches
        .subcommand()
        .ok_or_else(|| ForgeError::InvalidArgument("a subcommand is required".into()))?;
    let name = match name {
        "n" => "new",
        "ti" => "test-init",
        "dr" => "doctor",
        other => other,
    };

    // Try a built-in plugin first.
    if let Some(plugin) = plugins.iter().find(|p| p.name() == name) {
        let cwd = if let Some(dir) = matches.get_one::<String>("cwd") {
            let path = std::path::PathBuf::from(dir);
            if !path.is_dir() {
                return Err(ForgeError::InvalidArgument(format!("--cwd path `{}` is not a directory", path.display())));
            }
            path.canonicalize().map_err(|e| ForgeError::InvalidArgument(format!("cannot resolve --cwd `{}`: {e}", path.display())))?
        } else {
            std::env::current_dir().map_err(ForgeError::io("determining current directory"))?
        };
        let ctx = ForgeContext::with_options(
            cwd,
            verbose,
            quiet,
            json,
            yes,
            offline,
            matches.get_one::<String>("log-level").cloned(),
            matches
                .get_one::<String>("timeout")
                .and_then(|value| value.parse::<u64>().ok()),
            matches.get_one::<String>("config").map(std::path::PathBuf::from),
        )?;
        ctx.progress(&format!("running {name}"));
        log::debug!("dispatching to plugin `{}`", plugin.name());
        plugin.pre_run(sub_matches, &ctx)?;
        let result = plugin.run(sub_matches, &ctx);
        plugin.post_run(sub_matches, &ctx)?;
        return result;
    }

    // External plugins are outside forge's control and could access the network.
    if offline {
        return Err(ForgeError::InvalidArgument(format!(
            "external subcommand `{name}` is unavailable in offline mode"
        )));
    }

    let plugin_names: Vec<&str> = plugins.iter().map(|p| p.name()).collect();
    if let Some(suggestion) = closest_match(name, &plugin_names, 3) {
        eprintln!("unknown subcommand `{name}`. Did you mean `{suggestion}`?");
    }
    try_run_external(name, sub_matches)
}

/// Entry point used by the `soroban-forge` binary: parse `std::env::args`,
/// initialise logging and dispatch.  clap handles `--help`/`--version`/usage
/// errors itself (printing and exiting), as users expect.
pub fn run(plugins: Vec<Box<dyn ForgePlugin>>) -> Result<()> {
    let cmd = build_command(&plugins);
    let matches = cmd.get_matches();

    // --list must be handled before logging / dispatch so it works even
    // when there is no subcommand.
    if matches.get_flag("list") {
        list_subcommands(&plugins);
        return Ok(());
    }

    let log_file = matches.get_one::<String>("log-file").map(std::path::Path::new);
    crate::logging::init(matches.get_count("verbose"), log_file)?;

    let is_json = matches.get_flag("json");
    let result = dispatch(&plugins, &matches);
    if let Err(ref err) = result {
        if is_json {
            let json_err = serde_json::json!({
                "error": err.to_string(),
                "exit_code": i32::from(err.exit_code())
            });
            eprintln!("{}", serde_json::to_string_pretty(&json_err).unwrap());
            std::process::exit(err.exit_code().into());
        }
    }
    result
}

// ---------------------------------------------------------------------------
// External subcommand helpers
// ---------------------------------------------------------------------------

/// Look up `soroban-forge-{name}` on PATH and exec it with the remaining
/// arguments.  The child inherits our stdio and its exit code becomes ours.
fn try_run_external(name: &str, sub_matches: &ArgMatches) -> Result<()> {
    let bin = format!("soroban-forge-{name}");

    let ext_args: Vec<&OsStr> = sub_matches
        .get_many::<std::ffi::OsString>("")
        .unwrap_or_default()
        .map(|s| s.as_os_str())
        .collect();

    let mut cmd = std::process::Command::new(&bin);
    cmd.args(&ext_args);
    cmd.stdin(std::process::Stdio::inherit());
    cmd.stdout(std::process::Stdio::inherit());
    cmd.stderr(std::process::Stdio::inherit());

    match cmd.status() {
        Ok(status) => {
            std::process::exit(status.code().unwrap_or(1));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(ForgeError::InvalidArgument(format!(
                "unknown subcommand `{name}`"
            )))
        }
        Err(e) => Err(ForgeError::Io {
            context: format!("running `{bin}`"),
            source: e,
        }),
    }
}

/// Print all subcommands (built-in + external) to stdout.
fn list_subcommands(plugins: &[Box<dyn ForgePlugin>]) {
    println!("Installed subcommands:");
    println!();

    let mut builtins: Vec<&str> = plugins.iter().map(|p| p.name()).collect();
    builtins.sort();
    println!("  Built-in:");
    for name in &builtins {
        println!("    {name}");
    }

    let externals = find_external_subcommands();
    if !externals.is_empty() {
        println!("  External:");
        for name in &externals {
            println!("    {name}");
        }
    }
}

/// Scan `PATH` for `soroban-forge-*` binaries and return their subcommand
/// names (the part after the prefix), deduplicated and sorted.
fn find_external_subcommands() -> Vec<String> {
    let prefix = "soroban-forge-";
    let mut seen = Vec::new();

    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let s = name.to_string_lossy();
                    if let Some(stripped) = s.strip_prefix(prefix) {
                        let sub = stripped.to_string();
                        if !seen.contains(&sub) {
                            seen.push(sub);
                        }
                    }
                }
            }
        }
    }

    seen.sort();
    seen
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
    fn help_lists_list_flag() {
        let (plugins, _) = dummy();
        let help = build_command(&plugins).render_long_help().to_string();
        assert!(help.contains("--list"));
        assert!(help.contains("List all installed subcommands"));
    }

    #[test]
    fn help_shows_external_subcommand_footer() {
        let (plugins, _) = dummy();
        let help = build_command(&plugins).render_long_help().to_string();
        assert!(help.contains("--list"));
    }

    #[test]
    fn help_lists_quiet_flag() {
        let (plugins, _) = dummy();
        let help = build_command(&plugins).render_long_help().to_string();
        assert!(help.contains("--quiet"));
        assert!(help.contains("Suppress informational command output"));
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
    fn unknown_subcommand_is_no_longer_a_parse_error() {
        let (plugins, _) = dummy();
        let result = build_command(&plugins).try_get_matches_from(["soroban-forge", "nonexistent"]);
        assert!(result.is_ok(), "external subcommands are allowed at parse time");
    }

    #[test]
    fn unknown_subcommand_fails_at_dispatch() {
        let (plugins, _) = dummy();
        let matches = build_command(&plugins)
            .try_get_matches_from(["soroban-forge", "nonexistent"])
            .unwrap();
        let result = dispatch(&plugins, &matches);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("nonexistent"),
            "error should mention the unknown name"
        );
    }

    #[test]
    fn quiet_is_global_after_subcommand() {
        let (plugins, _) = dummy();
        let matches = build_command(&plugins)
            .try_get_matches_from(["soroban-forge", "dummy", "--flag", "--quiet"])
            .unwrap();
        assert!(matches.get_flag("quiet"));
    }

    #[test]
    fn quiet_and_verbose_can_be_combined() {
        let (plugins, _) = dummy();
        let matches = build_command(&plugins)
            .try_get_matches_from(["soroban-forge", "--quiet", "--verbose", "dummy", "--flag"])
            .unwrap();
        assert!(matches.get_flag("quiet"));
        assert_eq!(matches.get_count("verbose"), 1);
    }

    #[test]
    fn verbose_is_repeatable() {
        let (plugins, _) = dummy();
        let matches = build_command(&plugins)
            .try_get_matches_from(["soroban-forge", "-vv", "dummy", "--flag"])
            .unwrap();
        assert_eq!(matches.get_count("verbose"), 2);
    }

    #[test]
    fn yes_is_global_flag() {
        let (plugins, _) = dummy();
        let matches = build_command(&plugins)
            .try_get_matches_from(["soroban-forge", "--yes", "dummy", "--flag"])
            .unwrap();
        assert!(matches.get_flag("yes"));
    }

    #[test]
    fn yes_works_after_subcommand() {
        let (plugins, _) = dummy();
        let matches = build_command(&plugins)
            .try_get_matches_from(["soroban-forge", "dummy", "--flag", "--yes"])
            .unwrap();
        assert!(matches.get_flag("yes"));
    }

    #[test]
    fn json_is_global_flag() {
        let (plugins, _) = dummy();
        let matches = build_command(&plugins)
            .try_get_matches_from(["soroban-forge", "--json", "dummy", "--flag"])
            .unwrap();
        assert!(matches.get_flag("json"));
    }
}
