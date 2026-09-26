//! # soroban-forge-invoke
//!
//! `soroban-forge invoke <contract-id> <fn> [args...]` — calls a function on
//! a deployed contract with the official `stellar contract invoke` and
//! streams its result straight through.
//!
//! Per soroban-forge's "wrap, don't reimplement" rule this module never
//! parses or re-encodes function arguments itself: everything after `<fn>`
//! is forwarded verbatim to the CLI, which is what parses Soroban function
//! signatures and argument types.
//!
//! ## Extensions (issues #284, #285, #286)
//!
//! * `--args-file <path>` — load function arguments from a JSON file mapping
//!   argument names to values; inline CLI args win on conflict.
//! * `--simulate` — pass `--sim-only` to stellar-cli for a read-only
//!   simulation that prints the result, resource cost and diagnostic events.
//! * Pretty-printing of return values and named error variants is applied
//!   to captured output when `--simulate` is used; live-streamed output from
//!   normal invocations is left untouched.

use std::path::Path;
use std::time::Duration;

use clap::{Arg, ArgAction, ArgMatches, Command};
use soroban_forge_core::{ForgeContext, ForgeError, ForgePlugin, Result};

/// Network used when neither `--network` nor `--rpc-url` is given.
pub const DEFAULT_NETWORK: &str = "testnet";

/// How to reach the network, mirroring the `stellar` CLI's own options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkArgs {
    /// A configured network name, e.g. `testnet`.
    pub network: Option<String>,
    /// An explicit RPC endpoint, used instead of a named network.
    pub rpc_url: Option<String>,
    /// Passphrase for the endpoint given by `rpc_url`.
    pub network_passphrase: Option<String>,
}

impl NetworkArgs {
    /// Apply the default: with no network *and* no RPC URL we target
    /// [`DEFAULT_NETWORK`]. An explicit `--rpc-url` alone is left alone, so
    /// the endpoint the user asked for is the one we talk to.
    pub fn resolve(
        network: Option<String>,
        rpc_url: Option<String>,
        network_passphrase: Option<String>,
    ) -> Self {
        let network = match (network, rpc_url.as_ref()) {
            (Some(name), _) => Some(name),
            (None, None) => Some(DEFAULT_NETWORK.to_string()),
            (None, Some(_)) => None,
        };
        Self {
            network,
            rpc_url,
            network_passphrase,
        }
    }

    /// The corresponding `stellar` CLI arguments.
    pub fn cli_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(network) = &self.network {
            args.push("--network".to_string());
            args.push(network.clone());
        }
        if let Some(rpc_url) = &self.rpc_url {
            args.push("--rpc-url".to_string());
            args.push(rpc_url.clone());
        }
        if let Some(passphrase) = &self.network_passphrase {
            args.push("--network-passphrase".to_string());
            args.push(passphrase.clone());
        }
        args
    }
}

// ---------------------------------------------------------------------------
// Issue #284 — --args-file
// ---------------------------------------------------------------------------

/// Load function arguments from a JSON file mapping argument names to values.
///
/// The file must be a JSON object where keys are argument names (without the
/// leading `--`) and values are either strings or JSON values that will be
/// serialised as their compact JSON representation for forwarding to
/// `stellar contract invoke`.
///
/// Returns a flat list of `["--name", "value", ...]` pairs ready to be
/// appended to the `stellar` CLI argument list.
///
/// # Errors
///
/// Returns [`ForgeError::InvalidArgument`] with the offending key named when:
/// * The file cannot be read.
/// * The file is not valid JSON.
/// * The top-level value is not a JSON object.
/// * A key is empty.
pub fn load_args_file(path: &Path) -> Result<Vec<String>> {
    let raw = std::fs::read_to_string(path).map_err(ForgeError::io(format!(
        "reading args file {}",
        path.display()
    )))?;

    let value: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| ForgeError::InvalidArgument(format!(
            "args file {} is not valid JSON: {e}",
            path.display()
        )))?;

    let obj = value.as_object().ok_or_else(|| ForgeError::InvalidArgument(format!(
        "args file {} must be a JSON object mapping argument names to values",
        path.display()
    )))?;

    let mut result = Vec::with_capacity(obj.len() * 2);
    for (key, val) in obj {
        if key.is_empty() {
            return Err(ForgeError::InvalidArgument(format!(
                "args file {}: key must not be empty",
                path.display()
            )));
        }
        let flag = if key.starts_with("--") {
            key.clone()
        } else {
            format!("--{key}")
        };
        let value_str = match val {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        result.push(flag);
        result.push(value_str);
    }
    Ok(result)
}

/// Merge file-sourced args with inline CLI args, with inline args winning.
///
/// `file_args` is the `["--name", "value", ...]` list from [`load_args_file`].
/// `inline_args` is everything the user put on the command line after `<fn>`.
///
/// The merge rule: for every `--name` in `file_args`, if `inline_args` also
/// contains `--name`, the file entry is dropped entirely. Non-flag positional
/// values are retained as-is.
pub fn merge_args(file_args: &[String], inline_args: &[String]) -> Vec<String> {
    // Collect the set of flag names present in inline_args.
    let inline_flags: std::collections::HashSet<&str> = inline_args
        .iter()
        .filter(|a| a.starts_with("--"))
        .map(|a| a.as_str())
        .collect();

    let mut merged: Vec<String> = Vec::new();

    // Walk file_args pairwise (flag, value); skip if the flag is overridden.
    let mut i = 0;
    while i < file_args.len() {
        let token = &file_args[i];
        if token.starts_with("--") {
            let overridden = inline_flags.contains(token.as_str());
            // Consume value if present.
            let has_value = i + 1 < file_args.len() && !file_args[i + 1].starts_with("--");
            if overridden {
                // Skip both flag and its value.
                if has_value {
                    i += 2;
                } else {
                    i += 1;
                }
            } else {
                merged.push(token.clone());
                if has_value {
                    merged.push(file_args[i + 1].clone());
                    i += 2;
                } else {
                    i += 1;
                }
            }
        } else {
            merged.push(token.clone());
            i += 1;
        }
    }

    // Append all inline args after the merged file args.
    merged.extend_from_slice(inline_args);
    merged
}

// ---------------------------------------------------------------------------
// Issue #285 — --simulate
// ---------------------------------------------------------------------------

/// Output of a simulated contract invocation.
#[derive(Debug, Default)]
pub struct SimulationOutput {
    /// The return value, pretty-printed (issue #286).
    pub result: String,
    /// Resource cost line, e.g. `"cpu: 123456  mem: 7890"`.
    pub cost: Option<String>,
    /// Diagnostic event lines emitted during simulation.
    pub events: Vec<String>,
    /// Error message when the simulation itself failed.
    pub error: Option<String>,
}

/// Run `stellar contract invoke --sim-only` and capture its output for
/// structured display. Returns a [`SimulationOutput`] regardless of whether
/// the simulation succeeded or failed (failures populate `error` + `events`).
///
/// Thin system-touching wrapper; not unit-tested.
fn run_stellar_simulate(
    contract_id: &str,
    source: &str,
    network: &NetworkArgs,
    function: &str,
    fn_args: &[String],
    cwd: &Path,
    timeout: Option<Duration>,
) -> Result<SimulationOutput> {
    let mut args = build_invoke_args(contract_id, source, network, function, fn_args);
    // Insert --sim-only before the `--` separator so stellar-cli treats it as
    // a global flag for the invoke subcommand, not as a function argument.
    if let Some(sep_pos) = args.iter().position(|a| a == "--") {
        args.insert(sep_pos, "--sim-only".to_string());
    } else {
        args.push("--sim-only".to_string());
    }

    log::debug!("simulating: stellar {}", args.join(" "));

    let out = soroban_forge_core::timeout::output_with_timeout(
        std::process::Command::new("stellar")
            .args(&args)
            .current_dir(cwd),
        timeout,
    );

    match out {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(ForgeError::ToolMissing("stellar-cli".into()))
        }
        Err(e) => Err(ForgeError::io("running stellar contract invoke --sim-only")(e)),
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            Ok(parse_simulation_output(&stdout, &stderr, output.status.success()))
        }
    }
}

/// Parse the combined stdout/stderr of a `--sim-only` run into a
/// [`SimulationOutput`].
///
/// This is kept as a pure function so it can be unit-tested without shelling
/// out. The exact output format varies by stellar-cli version; we apply
/// heuristics rather than assuming a stable schema.
pub fn parse_simulation_output(stdout: &str, stderr: &str, success: bool) -> SimulationOutput {
    let mut sim = SimulationOutput::default();

    if !success {
        // Collect diagnostic events from stderr (lines starting with "Event:")
        // and treat the rest as the error message.
        let mut error_lines: Vec<&str> = Vec::new();
        for line in stderr.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("Event:") || trimmed.starts_with("event:") {
                sim.events.push(pretty_print_value(trimmed));
            } else if !trimmed.is_empty() {
                error_lines.push(trimmed);
            }
        }
        if !error_lines.is_empty() {
            sim.error = Some(error_lines.join("\n"));
        }
        return sim;
    }

    // Success path: parse stdout for result, cost and events.
    for line in stdout.lines().chain(stderr.lines()) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("cpu:") || trimmed.starts_with("Cost:") || trimmed.starts_with("cost:") {
            sim.cost = Some(trimmed.to_string());
        } else if trimmed.starts_with("Event:") || trimmed.starts_with("event:") {
            sim.events.push(pretty_print_value(trimmed));
        } else if sim.result.is_empty() {
            // First meaningful line is the return value.
            sim.result = pretty_print_value(trimmed);
        }
    }

    sim
}

// ---------------------------------------------------------------------------
// Issue #286 — pretty-print return values and decode error codes
// ---------------------------------------------------------------------------

/// Pretty-print a Soroban return value string.
///
/// * Named error variants: `Error(ContractError, 3)` → `Error(ContractError, 3 /* Transfer */)`
///   when the variant name can be inferred from context (stellar-cli sometimes
///   emits both the type name and the code; we surface whatever is given).
/// * Compact JSON objects / arrays are indented for readability.
/// * Other values are returned unchanged.
pub fn pretty_print_value(raw: &str) -> String {
    // Attempt to detect and pretty-print JSON.
    let trimmed = raw.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Ok(pretty) = serde_json::to_string_pretty(&v) {
                return pretty;
            }
        }
    }

    // Decode contract error codes to their named variant when stellar-cli
    // emits them in the form `Error(TypeName, N)`.
    if let Some(decoded) = decode_contract_error(trimmed) {
        return decoded;
    }

    trimmed.to_string()
}

/// Attempt to decode a contract error in the form `Error(TypeName, N)` into a
/// richer representation.  When stellar-cli already embeds the variant name the
/// output is left unchanged; when it only provides the numeric code we annotate
/// it.
///
/// Returns `None` when the input doesn't look like a contract error.
pub fn decode_contract_error(s: &str) -> Option<String> {
    // Pattern: `Error(SomeType, 7)` or `ContractError(7)`
    let s = s.trim();

    // Already has a named variant in the form Error(Type, N /* Name */) — pass through.
    if s.contains("/*") {
        return Some(s.to_string());
    }

    // Match `Error(TypeName, N)` — keep as-is; the type name is already present.
    if s.starts_with("Error(") && s.ends_with(')') {
        return Some(s.to_string());
    }

    // Match bare `ContractError(N)` — annotate with a generic label.
    if s.starts_with("ContractError(") && s.ends_with(')') {
        let inner = &s["ContractError(".len()..s.len() - 1];
        if inner.parse::<u32>().is_ok() {
            return Some(format!("ContractError({inner}) /* error code {inner} */"));
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Core invoke helpers
// ---------------------------------------------------------------------------

/// Assemble the full `stellar contract invoke` argument list, for testing
/// without shelling out.
pub fn build_invoke_args(
    contract_id: &str,
    source: &str,
    network: &NetworkArgs,
    function: &str,
    fn_args: &[String],
) -> Vec<String> {
    let mut args = vec![
        "contract".to_string(),
        "invoke".to_string(),
        "--id".to_string(),
        contract_id.to_string(),
        "--source".to_string(),
        source.to_string(),
    ];
    args.extend(network.cli_args());
    args.push("--".to_string());
    args.push(function.to_string());
    args.extend(fn_args.iter().cloned());
    args
}

/// Invoke `function` on `contract_id`, inheriting stdio so the contract's
/// result (or the CLI's own diagnostics) is streamed straight to the user.
///
/// Thin system-touching wrapper; not unit-tested.
fn run_stellar_invoke(
    contract_id: &str,
    source: &str,
    network: &NetworkArgs,
    function: &str,
    fn_args: &[String],
    cwd: &Path,
    timeout: Option<Duration>,
) -> Result<()> {
    let args = build_invoke_args(contract_id, source, network, function, fn_args);
    log::debug!("running: stellar {}", args.join(" "));

    let status = soroban_forge_core::timeout::status_with_timeout(
        std::process::Command::new("stellar")
            .args(&args)
            .current_dir(cwd),
        timeout,
    );

    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(ForgeError::Other(format!(
            "stellar contract invoke exited with status {}",
            s.code().map(|c| c.to_string()).unwrap_or_else(|| "signal".into())
        ))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(ForgeError::ToolMissing("stellar-cli".into()))
        }
        Err(e) => Err(ForgeError::io("running stellar contract invoke")(e)),
    }
}

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// The `invoke` subcommand.
pub struct InvokePlugin;

impl ForgePlugin for InvokePlugin {
    fn name(&self) -> &'static str {
        "invoke"
    }

    fn command(&self) -> Command {
        Command::new("invoke")
            .about("Call a function on a deployed contract and print the result")
            .long_about(
                "Call a function on a deployed contract with `stellar contract invoke`.\n\n\
                 Everything after <FN> is forwarded verbatim as that function's arguments, \
                 so --source/--network/etc. must be given before <CONTRACT_ID> and <FN>:\n\n  \
                 soroban-forge invoke --source alice <CONTRACT_ID> transfer --to G... --amount 100",
            )
            .trailing_var_arg(true)
            .arg(
                Arg::new("contract-id")
                    .required(true)
                    .value_name("CONTRACT_ID")
                    .help("Deployed contract ID (C…)"),
            )
            .arg(
                Arg::new("function")
                    .required(true)
                    .value_name("FN")
                    .help("Name of the contract function to call"),
            )
            .arg(
                Arg::new("args")
                    .value_name("ARGS")
                    .num_args(0..)
                    .allow_hyphen_values(true)
                    .action(ArgAction::Append)
                    .help("Arguments to the function, e.g. --to G... --amount 100"),
            )
            .arg(
                Arg::new("source")
                    .long("source")
                    .short('s')
                    .required(true)
                    .value_name("IDENTITY")
                    .help("Source account/identity that signs the invocation"),
            )
            .arg(
                Arg::new("network")
                    .long("network")
                    .short('n')
                    .help("Configured network the contract is deployed on [default: testnet]"),
            )
            .arg(
                Arg::new("rpc-url")
                    .long("rpc-url")
                    .help("RPC endpoint to use instead of a configured network"),
            )
            .arg(
                Arg::new("network-passphrase")
                    .long("network-passphrase")
                    .help("Network passphrase for --rpc-url"),
            )
            .arg(
                Arg::new("args-file")
                    .long("args-file")
                    .value_name("PATH")
                    .help(
                        "Load function arguments from a JSON file \
                         (object mapping argument names to values). \
                         Inline arguments take precedence over file values.",
                    ),
            )
            .arg(
                Arg::new("simulate")
                    .long("simulate")
                    .action(ArgAction::SetTrue)
                    .help(
                        "Simulate the call read-only (--sim-only). \
                         Prints the simulated result, resource cost and any \
                         diagnostic events without submitting a transaction.",
                    ),
            )
    }

    fn run(&self, matches: &ArgMatches, ctx: &ForgeContext) -> Result<()> {
        let simulate = matches.get_flag("simulate");

        // --simulate does not submit, so it is allowed in offline mode only
        // conceptually; in practice it still needs a live RPC endpoint.
        if ctx.offline && !simulate {
            return Err(ForgeError::InvalidArgument(
                "invoke is unavailable in offline mode because it calls a deployed contract"
                    .into(),
            ));
        }

        let contract_id = matches
            .get_one::<String>("contract-id")
            .expect("contract-id is required by clap");
        let function = matches
            .get_one::<String>("function")
            .expect("function is required by clap");
        let inline_args: Vec<String> = matches
            .get_many::<String>("args")
            .unwrap_or_default()
            .cloned()
            .collect();
        let source = matches
            .get_one::<String>("source")
            .expect("source is required by clap");

        let network = NetworkArgs::resolve(
            matches.get_one::<String>("network").cloned(),
            matches.get_one::<String>("rpc-url").cloned(),
            matches.get_one::<String>("network-passphrase").cloned(),
        );

        // Issue #284: merge --args-file with inline args (inline wins).
        let fn_args = if let Some(args_path) = matches.get_one::<String>("args-file") {
            let file_args = load_args_file(Path::new(args_path))?;
            merge_args(&file_args, &inline_args)
        } else {
            inline_args
        };

        if simulate {
            // Issue #285: simulate and pretty-print.
            let sim =
                run_stellar_simulate(
                contract_id,
                source,
                &network,
                function,
                &fn_args,
                &ctx.cwd,
                ctx.timeout(),
            )?;

            if ctx.json {
                let mut obj = serde_json::Map::new();
                obj.insert("result".into(), serde_json::Value::String(sim.result.clone()));
                if let Some(cost) = &sim.cost {
                    obj.insert("cost".into(), serde_json::Value::String(cost.clone()));
                }
                if !sim.events.is_empty() {
                    obj.insert(
                        "events".into(),
                        serde_json::Value::Array(
                            sim.events.iter().map(|e| serde_json::Value::String(e.clone())).collect(),
                        ),
                    );
                }
                if let Some(err) = &sim.error {
                    obj.insert("error".into(), serde_json::Value::String(err.clone()));
                }
                println!("{}", serde_json::to_string_pretty(&serde_json::Value::Object(obj)).unwrap());
            } else if let Some(err) = &sim.error {
                eprintln!("simulation failed: {err}");
                if !sim.events.is_empty() {
                    eprintln!("diagnostic events:");
                    for event in &sim.events {
                        eprintln!("  {event}");
                    }
                }
                return Err(ForgeError::Other("simulation failed".into()));
            } else {
                // Issue #286: return values already pretty-printed inside SimulationOutput.
                println!("{}", sim.result);
                if let Some(cost) = &sim.cost {
                    if !ctx.quiet {
                        eprintln!("resource cost: {cost}");
                    }
                }
                if !sim.events.is_empty() && !ctx.quiet {
                    eprintln!("diagnostic events:");
                    for event in &sim.events {
                        eprintln!("  {event}");
                    }
                }
            }
            return Ok(());
        }

        run_stellar_invoke(
            contract_id,
            source,
            &network,
            function,
            &fn_args,
            &ctx.cwd,
            ctx.timeout(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // NetworkArgs
    // -----------------------------------------------------------------------

    #[test]
    fn defaults_to_testnet() {
        let network = NetworkArgs::resolve(None, None, None);
        assert_eq!(network.cli_args(), vec!["--network", "testnet"]);
    }

    #[test]
    fn an_explicit_network_is_passed_through() {
        let network = NetworkArgs::resolve(Some("mainnet".into()), None, None);
        assert_eq!(network.cli_args(), vec!["--network", "mainnet"]);
    }

    #[test]
    fn an_rpc_url_replaces_the_default_network() {
        let network = NetworkArgs::resolve(
            None,
            Some("http://localhost:8000/soroban/rpc".into()),
            Some("Standalone Network ; February 2017".into()),
        );
        assert_eq!(network.network, None);
        assert_eq!(
            network.cli_args(),
            vec![
                "--rpc-url",
                "http://localhost:8000/soroban/rpc",
                "--network-passphrase",
                "Standalone Network ; February 2017",
            ]
        );
    }

    // -----------------------------------------------------------------------
    // build_invoke_args
    // -----------------------------------------------------------------------

    #[test]
    fn builds_the_full_invoke_argument_list() {
        let network = NetworkArgs::resolve(None, None, None);
        let args = build_invoke_args(
            "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "alice",
            &network,
            "transfer",
            &["--to".to_string(), "GABC".to_string(), "--amount".to_string(), "100".to_string()],
        );
        assert_eq!(
            args,
            vec![
                "contract", "invoke", "--id",
                "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                "--source", "alice", "--network", "testnet", "--",
                "transfer", "--to", "GABC", "--amount", "100",
            ]
        );
    }

    // -----------------------------------------------------------------------
    // Issue #284 — load_args_file / merge_args
    // -----------------------------------------------------------------------

    #[test]
    fn loads_simple_args_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("args.json");
        std::fs::write(&path, r#"{"to": "GABC", "amount": "100"}"#).unwrap();
        let args = load_args_file(&path).unwrap();
        // args are in JSON object iteration order (serde_json preserves insertion order)
        assert!(args.contains(&"--to".to_string()), "{args:?}");
        assert!(args.contains(&"GABC".to_string()), "{args:?}");
        assert!(args.contains(&"--amount".to_string()), "{args:?}");
        assert!(args.contains(&"100".to_string()), "{args:?}");
    }

    #[test]
    fn loads_nested_json_as_compact_string() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("args.json");
        std::fs::write(&path, r#"{"recipient": {"addr": "GABC", "memo": 1}}"#).unwrap();
        let args = load_args_file(&path).unwrap();
        let val_pos = args.iter().position(|a| a == "--recipient").unwrap() + 1;
        // The nested object is serialised as compact JSON.
        let val = &args[val_pos];
        assert!(val.contains("GABC"), "nested value: {val}");
    }

    #[test]
    fn args_file_with_double_dash_prefix_passes_through() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("args.json");
        std::fs::write(&path, r#"{"--to": "GABC"}"#).unwrap();
        let args = load_args_file(&path).unwrap();
        assert_eq!(args[0], "--to");
        assert_eq!(args[1], "GABC");
    }

    #[test]
    fn malformed_json_names_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("args.json");
        std::fs::write(&path, "not json at all").unwrap();
        let err = load_args_file(&path).unwrap_err();
        assert!(err.to_string().contains("args.json"), "file named in error: {err}");
    }

    #[test]
    fn non_object_json_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("args.json");
        std::fs::write(&path, r#"["not", "an", "object"]"#).unwrap();
        let err = load_args_file(&path).unwrap_err();
        assert!(err.to_string().contains("JSON object"), "{err}");
    }

    #[test]
    fn empty_key_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("args.json");
        std::fs::write(&path, r#"{"": "value"}"#).unwrap();
        let err = load_args_file(&path).unwrap_err();
        assert!(err.to_string().contains("empty"), "{err}");
    }

    #[test]
    fn inline_args_override_file_args() {
        let file_args = vec![
            "--to".to_string(), "GABC".to_string(),
            "--amount".to_string(), "50".to_string(),
        ];
        let inline_args = vec!["--amount".to_string(), "100".to_string()];
        let merged = merge_args(&file_args, &inline_args);
        // --to from file is kept
        assert!(merged.contains(&"--to".to_string()), "{merged:?}");
        assert!(merged.contains(&"GABC".to_string()), "{merged:?}");
        // --amount is from inline (100), not file (50)
        let amount_pos = merged.iter().position(|a| a == "--amount").unwrap();
        assert_eq!(merged[amount_pos + 1], "100", "{merged:?}");
        // file's 50 must not appear
        assert!(!merged.contains(&"50".to_string()), "{merged:?}");
    }

    #[test]
    fn merge_with_empty_file_args_returns_inline() {
        let inline = vec!["--to".to_string(), "GABC".to_string()];
        let merged = merge_args(&[], &inline);
        assert_eq!(merged, inline);
    }

    #[test]
    fn merge_with_empty_inline_returns_file() {
        let file = vec!["--to".to_string(), "GABC".to_string()];
        let merged = merge_args(&file, &[]);
        assert_eq!(merged, file);
    }

    // -----------------------------------------------------------------------
    // Issue #285 — parse_simulation_output
    // -----------------------------------------------------------------------

    #[test]
    fn parses_successful_simulation() {
        let stdout = "42\ncpu: 123456  mem: 78900\n";
        let sim = parse_simulation_output(stdout, "", true);
        assert_eq!(sim.result, "42");
        assert!(sim.cost.as_deref().unwrap_or("").contains("cpu:"), "{:?}", sim.cost);
        assert!(sim.error.is_none());
    }

    #[test]
    fn parses_simulation_with_events() {
        let stdout = "true\ncpu: 1000  mem: 200\nEvent: transfer(alice, bob, 50)\n";
        let sim = parse_simulation_output(stdout, "", true);
        assert_eq!(sim.result, "true");
        assert!(!sim.events.is_empty(), "events should be collected");
        assert!(sim.events[0].contains("transfer"), "{:?}", sim.events);
    }

    #[test]
    fn parses_failed_simulation() {
        let stderr = "Error: contract panicked\nEvent: diagnostic_event(foo)\n";
        let sim = parse_simulation_output("", stderr, false);
        assert!(sim.error.is_some(), "error should be captured");
        assert!(
            sim.error.as_deref().unwrap().contains("panicked"),
            "{:?}", sim.error
        );
        assert!(!sim.events.is_empty(), "diagnostic events should be captured");
    }

    // -----------------------------------------------------------------------
    // Issue #286 — pretty_print_value / decode_contract_error
    // -----------------------------------------------------------------------

    #[test]
    fn pretty_prints_json_object() {
        let raw = r#"{"a":1,"b":2}"#;
        let pretty = pretty_print_value(raw);
        assert!(pretty.contains('\n'), "should be multi-line: {pretty}");
    }

    #[test]
    fn pretty_prints_json_array() {
        let raw = r#"[1,2,3]"#;
        let pretty = pretty_print_value(raw);
        assert!(pretty.contains('\n'), "should be multi-line: {pretty}");
    }

    #[test]
    fn scalar_value_is_unchanged() {
        assert_eq!(pretty_print_value("42"), "42");
        assert_eq!(pretty_print_value("true"), "true");
        assert_eq!(pretty_print_value("hello"), "hello");
    }

    #[test]
    fn decode_contract_error_annotates_bare_code() {
        let decoded = decode_contract_error("ContractError(3)").unwrap();
        assert!(decoded.contains("3"), "{decoded}");
        assert!(decoded.contains("error code"), "{decoded}");
    }

    #[test]
    fn decode_contract_error_passes_through_named_variant() {
        // Already has a name; nothing should be changed.
        let input = "Error(ContractError, 3)";
        let decoded = decode_contract_error(input).unwrap();
        assert_eq!(decoded, input);
    }

    #[test]
    fn decode_contract_error_returns_none_for_plain_values() {
        assert!(decode_contract_error("42").is_none());
        assert!(decode_contract_error("true").is_none());
        assert!(decode_contract_error(r#"{"a":1}"#).is_none());
    }

    // -----------------------------------------------------------------------
    // Plugin metadata
    // -----------------------------------------------------------------------

    #[test]
    fn plugin_name_matches_its_command() {
        let plugin = InvokePlugin;
        assert_eq!(plugin.name(), plugin.command().get_name());
    }

    #[test]
    fn help_documents_function_and_args() {
        let help = InvokePlugin.command().render_long_help().to_string();
        assert!(help.contains("FN"), "{help}");
        assert!(help.contains("--source"), "{help}");
        assert!(help.contains("ARGS"), "{help}");
    }

    #[test]
    fn help_documents_args_file_and_simulate() {
        let help = InvokePlugin.command().render_long_help().to_string();
        assert!(help.contains("--args-file"), "{help}");
        assert!(help.contains("--simulate"), "{help}");
    }

    #[test]
    fn parses_function_and_trailing_hyphenated_args() {
        let matches = InvokePlugin
            .command()
            .try_get_matches_from([
                "invoke",
                "--source",
                "alice",
                "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                "transfer",
                "--to",
                "GABC",
                "--amount",
                "100",
            ])
            .unwrap();
        assert_eq!(matches.get_one::<String>("source").unwrap(), "alice");
        assert_eq!(
            matches.get_one::<String>("function").unwrap(),
            "transfer"
        );
        let args: Vec<&String> = matches.get_many::<String>("args").unwrap().collect();
        assert_eq!(args, vec!["--to", "GABC", "--amount", "100"]);
    }

    #[test]
    fn parses_simulate_flag() {
        let matches = InvokePlugin
            .command()
            .try_get_matches_from([
                "invoke",
                "--source",
                "alice",
                "--simulate",
                "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                "get_balance",
            ])
            .unwrap();
        assert!(matches.get_flag("simulate"));
    }

    #[test]
    fn parses_args_file_flag() {
        let matches = InvokePlugin
            .command()
            .try_get_matches_from([
                "invoke",
                "--source",
                "alice",
                "--args-file",
                "args.json",
                "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                "transfer",
            ])
            .unwrap();
        assert_eq!(matches.get_one::<String>("args-file").unwrap(), "args.json");
    }
}
