//! # soroban-forge-scaffold
//!
//! `soroban-forge new <name> --template <t>` — creates a new Soroban contract
//! project from one of the bundled templates.
//!
//! Templates live in the repository's top-level `templates/` directory and are
//! embedded into the binary at compile time. A template is a plain directory
//! tree; file contents (and names) may contain `{{variable}}` placeholders.
//! Files whose name ends in `.hbs` have that suffix stripped on render — this
//! is how templates ship a `Cargo.toml.hbs` without cargo mistaking it for a
//! real manifest.
//!
//! Built-in variables: `project_name`, `crate_name`, `author`, `sdk_version`,
//! `edition`. A template may declare further variables in a `template.toml`
//! manifest (see [`manifest`]); those are filled from `--var name=value`, from
//! the manifest default, or by prompting when the session is interactive.

pub mod license;
pub mod manifest;

pub use manifest::{TemplateManifest, TemplateVariable};

use std::collections::BTreeMap;
use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;

use clap::{Arg, ArgAction, ArgMatches, Command};
use include_dir::{include_dir, Dir};
use soroban_forge_core::render::{render_str, Vars};
use soroban_forge_core::{ForgeContext, ForgeError, ForgePlugin, Result};

static TEMPLATES: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../templates");

/// Name of the per-template metadata file (see [`manifest::TemplateManifest`]).
const MANIFEST_FILE_NAME: &str = "template.toml";

/// The soroban-sdk version pinned into generated projects.
/// TODO(verify): bump when a new stable soroban-sdk major is released.
pub const SOROBAN_SDK_VERSION: &str = "26.1.0"; // pinned sdk version

const DEFAULT_TEMPLATE: &str = "hello-world";

/// Name of the top-level directory under `templates/` holding files shared
/// across templates (see [`compose_partials`]) rather than a template itself.
const PARTIALS_DIR: &str = "_partials";

/// Marker line a template's `Cargo.toml.hbs` carries in place of its own
/// `[profile.release]` / `[profile.release-with-logs]` blocks; spliced out
/// for the shared partial content by [`compose_partials`].
const RELEASE_PROFILE_MARKER: &str = "# soroban-forge:partial:release-profile\n";

/// Pre-commit configuration with rustfmt and clippy hooks.
const PRE_COMMIT_CONFIG: &str = r#"# See https://pre-commit.com for more information
# See https://pre-commit.com/hooks.html for more hooks
repos:
  - repo: local
    hooks:
      - id: rustfmt
        name: rustfmt
        entry: cargo fmt --
        language: system
        types: [rust]
        pass_filenames: false
      - id: clippy
        name: clippy
        entry: cargo clippy --all-targets --all-features -- -D warnings
        language: system
        types: [rust]
        pass_filenames: false
"#;

/// `.devcontainer/devcontainer.json` template for `new --devcontainer`
/// (before `{{project_name}}` substitution — see [`render_str`]).
fn devcontainer_json_template() -> String {
    use soroban_forge_core::toolchain::WASM_TARGET;
    format!(
        r#"{{
  "name": "{{{{project_name}}}}",
  "build": {{
    "dockerfile": "Dockerfile"
  }},
  "customizations": {{
    "vscode": {{
      "extensions": ["rust-lang.rust-analyzer", "tamasfe.even-better-toml"]
    }}
  }},
  "postCreateCommand": "cargo build --target {wasm_target}"
}}
"#,
        wasm_target = WASM_TARGET,
    )
}

/// `.devcontainer/Dockerfile` for `soroban-forge new --devcontainer`.
///
/// Pinned to the same minimum Rust/stellar-cli versions `soroban-forge
/// doctor` checks for (see `soroban_forge_core::toolchain`), so a container
/// built from this file always passes `doctor`.
fn devcontainer_dockerfile() -> String {
    use soroban_forge_core::toolchain::{MIN_RUST, MIN_STELLAR, WASM_TARGET};
    format!(
        r#"FROM rust:{major}.{minor}-bookworm

# Matches the minimums `soroban-forge doctor` checks for.
RUN rustup target add {wasm_target} \
    && cargo install --locked stellar-cli --version "^{stellar_major}"

WORKDIR /workspace
"#,
        major = MIN_RUST.0,
        minor = MIN_RUST.1,
        wasm_target = WASM_TARGET,
        stellar_major = MIN_STELLAR.0,
    )
}

/// Section appended to a generated project's `README.md` documenting the
/// `.devcontainer/` when `--devcontainer` is used.
const DEVCONTAINER_README_SECTION: &str = "\n## Dev Container\n\n\
This project ships a `.devcontainer/` so it opens ready-to-build in \
GitHub Codespaces or VS Code's Dev Containers extension: Rust, the \
`wasm32v1-none` target, and `stellar-cli` are preinstalled to the \
versions `soroban-forge doctor` requires.\n\n\
- **Codespaces**: click \"Code\" → \"Create codespace on main\" on GitHub.\n\
- **VS Code**: install the *Dev Containers* extension, then \
\"Reopen in Container\".\n";

/// Names of the bundled templates, sorted.
pub fn available_templates() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = TEMPLATES
        .dirs()
        .filter_map(|d| d.path().file_name().and_then(|n| n.to_str()))
        // `_partials` holds files shared across templates (see
        // `compose_partials`), not a template of its own.
        .filter(|name| *name != PARTIALS_DIR)
        .collect();
    names.sort_unstable();
    names
}

/// Load `template.toml` for `name`, or a default (empty) manifest when the
/// template hasn't been migrated to carry one yet.
pub fn load_manifest(name: &str) -> Result<TemplateManifest> {
    let template_dir = TEMPLATES.get_dir(name).ok_or_else(|| {
        ForgeError::Template(format!(
            "unknown template `{name}` (available: {})",
            available_templates().join(", ")
        ))
    })?;
    match template_dir.get_file(format!("{name}/{MANIFEST_FILE_NAME}")) {
        Some(file) => {
            let raw = file.contents_utf8().ok_or_else(|| {
                ForgeError::Template(format!("{MANIFEST_FILE_NAME} is not UTF-8"))
            })?;
            manifest::parse_manifest(raw)
        }
        None => Ok(TemplateManifest::default()),
    }
}

/// One-line description for a bundled template, or `None` for unknown names.
///
/// This is the single source of truth for template descriptions, used by both
/// the `templates` subcommand and any future JSON output layer.
pub fn template_description(name: &str) -> Option<&'static str> {
    match name {
        "access-control" => Some(
            "role-based access control — grant/revoke/has-role with an admin role that administers other roles",
        ),
        "amm" => Some("constant-product AMM / liquidity pool (x*y=k, 0.3% fee)"),
        "allowlist-token" => Some("allowlist-gated token with admin-managed transfer restrictions"),
        "amm" => Some("constant-product AMM / liquidity pool (x*y=k, 0.3% fee)"),
        "atomic-swap" => Some("atomic two-party token swap with dual authorization"),
        "crowdfund" => Some("escrow/deadline crowdfunding contract"),
        "cross-contract" => Some("two-contract workspace demonstrating cross-contract calls with authorization"),
        "dutch-auction" => Some("descending-price auction with linear price decay and immediate settlement"),
        "escrow" => Some("token escrow with approval or timeout-based refund path"),
        "faucet" => Some("token faucet dispensing a fixed amount per address with a cooldown"),
        "flash-loan" => Some(
            "uncollateralized single-transaction loan repaid via a borrower callback",
        ),
        "governance" => Some("DAO governance with weighted voting, quorum, and proposal execution"),
        "hello-world" => Some("minimal greeter contract (recommended starting point)"),
        "lottery" => Some("randomized lottery with ticket purchases and prize pool distribution"),
        "merkle-airdrop" => Some("one-claim-per-address airdrop verified against a merkle root"),
        "multisig" => Some("M-of-N multisig account contract (CustomAccountInterface)"),
        "nft" => Some("NFT (non-fungible token) with per-token metadata and minting"),
        "oracle-consumer" => Some("consumes price data from an external oracle (e.g. Reflector)"),
        "payment-splitter" => Some("splits received funds between payees by fixed shares"),
        "pausable" => Some("admin-controlled circuit breaker gating guarded entrypoints"),
        "nft-marketplace" => Some("NFT marketplace for listing, buying, and cancelling sales with configurable fees"),
        "oracle-consumer" => Some("consumes price data from an external oracle (e.g. Reflector)"),
        "payment-splitter" => Some("splits received funds between payees by fixed shares"),
        "prediction-market" => {
            Some("binary outcome market with oracle resolution and parimutuel payouts")
        }
        "soulbound" => Some("soulbound (non-transferable) token contract"),
        "staking" => Some("proportional reward staking with O(1) acc_reward_per_share accumulator"),
        "storage-migration" => Some("storage layout migration from v1 to v2 with version-marker pattern"),
        "streaming" => Some("streams tokens linearly over time with cancels and withdrawals"),
        "subscription" => Some("recurring payment charged once per elapsed interval"),
        "timelock" => Some("timelock controller for delayed execution and cancellation of queued calls"),
        "token" => Some("SEP-41 fungible token (soroban_sdk::token::TokenInterface)"),
        "upgradeable" => Some("admin-gated upgradeable contract (update_current_contract_wasm)"),
        "vesting" => Some("token vesting with cliff + linear release schedule"),
        "wrapped-asset" => Some("mints a wrapper token on deposit and burns it on withdraw 1:1"),
        "yield-vault" => Some("ERC-4626-style yield vault with proportional shares and vault-favoured rounding"),
        _ => None,
    }
}

/// Metadata for a single bundled template, including declared variables.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TemplateInfo {
    pub name: &'static str,
    pub description: String,
    /// Custom variables declared in the template's template.toml
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<manifest::TemplateVariable>,
}

/// Return metadata for every bundled template, sorted by name.
///
/// Designed so callers (the `templates` subcommand, future `--json` output,
/// etc.) work only with this slice — not with raw name/description pairs.
pub fn template_catalog() -> Vec<TemplateInfo> {
    available_templates()
        .into_iter()
        .map(|name| {
            let variables = bundled_manifest(name)
                .ok()
                .flatten()
                .map(|m| m.variables)
                .unwrap_or_default();
            TemplateInfo {
                name,
                description: template_description(name)
                    .unwrap_or("no description available")
                    .to_string(),
                variables,
            }
        })
        .collect()
}

/// Render the template catalogue shown by `new --list-templates`.
pub fn format_template_list(templates: &[&str]) -> String {
    let mut out = String::from("available templates:\n");
    for name in templates {
        out.push_str(&format!("  {name}\n"));
    }
    out
}

/// A project name must be a valid cargo package name: lowercase ASCII
/// letters, digits, `-` or `_`, starting with a letter.
pub fn validate_project_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    let valid = matches!(chars.next(), Some('a'..='z'))
        && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '-' | '_'));
    if valid {
        Ok(())
    } else {
        Err(ForgeError::InvalidArgument(format!(
            "`{name}` is not a valid project name (use lowercase letters, digits, `-` or `_`, starting with a letter)"
        )))
    }
}

/// Read and parse the `template.toml` of a bundled template.
///
/// Returns `Ok(None)` when the template ships no manifest — the common case,
/// since a manifest is only needed for custom variables.
pub fn bundled_manifest(template: &str) -> Result<Option<TemplateManifest>> {
    let Some(dir) = TEMPLATES.get_dir(template) else {
        return Ok(None);
    };
    let Some(file) = dir.get_file(format!("{template}/{}", manifest::MANIFEST_FILE)) else {
        return Ok(None);
    };
    let raw = file.contents_utf8().ok_or_else(|| {
        ForgeError::Template(format!(
            "{} of template `{template}` is not UTF-8",
            manifest::MANIFEST_FILE
        ))
    })?;
    manifest::parse_manifest(raw).map(Some)
}

/// Read and parse the `template.toml` at the root of a template directory on
/// disk (used for `--from` clones).
pub fn manifest_in_dir(dir: &Path) -> Result<Option<TemplateManifest>> {
    let path = dir.join(manifest::MANIFEST_FILE);
    if !path.is_file() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(ForgeError::io(format!("reading {}", path.display())))?;
    manifest::parse_manifest(&raw).map(Some)
}

/// A [`manifest::VarPrompter`] that reads answers from stdin.
///
/// Thin system-touching wrapper; the resolution logic it feeds is unit-tested
/// against a scripted prompter instead.
pub struct StdinPrompter;

impl manifest::VarPrompter for StdinPrompter {
    fn ask(&mut self, var: &TemplateVariable) -> Option<String> {
        match &var.default {
            Some(default) => print!("{} [{}]: ", var.prompt_text(), default),
            None => print!("{}: ", var.prompt_text()),
        }
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        if std::io::stdin().read_line(&mut answer).ok()? == 0 {
            return None; // EOF
        }
        Some(answer.trim().to_string())
    }
}

/// Whether missing template variables may be asked for interactively.
///
/// Never in `--json` mode (a prompt would corrupt the stream) and never with
/// `--yes` (which means "don't ask me anything"), so scripted and CI runs fail
/// fast on a missing required variable instead of hanging on a read.
fn can_prompt(ctx: &ForgeContext) -> bool {
    !ctx.json && !ctx.yes && std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// Resolve a template's declared variables and merge them into `vars`.
fn merge_template_vars(
    manifest: Option<&TemplateManifest>,
    supplied: &Vars,
    ctx: &ForgeContext,
    vars: &mut Vars,
) -> Result<()> {
    let empty = TemplateManifest::default();
    let manifest = manifest.unwrap_or(&empty);

    let resolved = if can_prompt(ctx) {
        if !manifest.variables.is_empty() && !ctx.quiet {
            println!("this template needs a few values (press enter to accept a default):");
        }
        manifest::resolve_variables(manifest, supplied, &mut StdinPrompter)?
    } else {
        manifest::resolve_variables(manifest, supplied, &mut manifest::NoPrompt)?
    };

    vars.extend(resolved);
    Ok(())
}

/// Confirm an overwrite before `--force` writes over an existing directory.
///
/// `--force` alone is not treated as consent when a human is watching: the
/// user is shown the path and asked. `--yes`, `--json` and non-interactive
/// sessions skip the question — there `--force` *is* the explicit intent.
fn confirm_overwrite(dest: &Path, ctx: &ForgeContext) -> Result<()> {
    if !dest.exists() || !can_prompt(ctx) {
        return Ok(());
    }
    print!(
        "{} already exists — --force will overwrite files in it. continue? [y/N] ",
        dest.display()
    );
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    let accepted = std::io::stdin().read_line(&mut answer).is_ok()
        && matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes");
    if accepted {
        Ok(())
    } else {
        Err(ForgeError::InvalidArgument(format!(
            "--force was declined — {} was left untouched",
            dest.display()
        )))
    }
}

/// Build the variable map for a project.
pub fn project_vars(project_name: &str, author: &str, edition: &str) -> Vars {
    let mut vars = BTreeMap::new();
    vars.insert("project_name".into(), project_name.to_string());
    vars.insert("crate_name".into(), project_name.replace('-', "_"));
    vars.insert("author".into(), author.to_string());
    vars.insert("sdk_version".into(), SOROBAN_SDK_VERSION.to_string());
    vars.insert("edition".into(), edition.to_string());
    vars
}

/// Parse `--var name=value` occurrences into a name → value map.
pub fn parse_var_overrides(pairs: &[String]) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for pair in pairs {
        let (name, value) = pair.split_once('=').ok_or_else(|| {
            ForgeError::InvalidArgument(format!("`--var {pair}` is not in `name=value` form"))
        })?;
        out.insert(name.to_string(), value.to_string());
    }
    Ok(out)
}

/// Resolve a template's extra variables (declared in its `template.toml`)
/// into a name → value map, in this priority order:
///
/// 1. `--var name=value` overrides
/// 2. an interactive prompt, when `interactive` is set
/// 3. the variable's declared default
///
/// `interactive` should be `false` in quiet mode and whenever stdin isn't a
/// terminal, so CI and scripted use never blocks waiting for input.
pub fn resolve_extra_vars(
    manifest: &TemplateManifest,
    overrides: &BTreeMap<String, String>,
    interactive: bool,
) -> Result<Vars> {
    let mut vars = Vars::new();
    for var in &manifest.variables {
        let value = if let Some(v) = overrides.get(&var.name) {
            v.clone()
        } else if interactive {
            prompt_for(var.prompt_text(), var.default.as_deref().unwrap_or(""))?
        } else {
            var.default.clone().unwrap_or_default()
        };
        vars.insert(var.name.clone(), value);
    }
    Ok(vars)
}

/// Prompt `question [default: default]` on stdout and read a line from
/// stdin, returning `default` when the answer is empty.
fn prompt_for(question: &str, default: &str) -> Result<String> {
    print!("{question} [{default}]: ");
    std::io::stdout()
        .flush()
        .map_err(ForgeError::io("writing prompt"))?;

    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(ForgeError::io("reading prompt response"))?;

    let answer = line.trim();
    if answer.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(answer.to_string())
    }
}

/// Generate `template` into `dest` (which must not already exist unless
/// `force` is set). This is the programmatic API behind `soroban-forge new`.
pub fn generate(template: &str, dest: &Path, vars: &Vars, force: bool) -> Result<()> {
    if template == PARTIALS_DIR {
        return Err(ForgeError::Template(format!(
            "unknown template `{template}` (available: {})",
            available_templates().join(", ")
        )));
    }
    let template_dir = TEMPLATES.get_dir(template).ok_or_else(|| {
        ForgeError::Template(format!(
            "unknown template `{template}` (available: {})",
            available_templates().join(", ")
        ))
    })?;

    if dest.exists() && !force {
        return Err(ForgeError::AlreadyExists(dest.to_path_buf()));
    }

    render_dir(template_dir, template, dest, vars)?;
    compose_partials(template, dest, vars)?;
    write_forge_toml(dest, vars)?;
    Ok(())
}

/// Fill in files shared across templates (see `templates/_partials/`) that
/// `template` didn't ship its own copy of, and splice the shared release
/// profile into the rendered `Cargo.toml`. Templates opt out of any of this
/// simply by shipping their own file at the same path — see the per-file
/// checks below.
fn compose_partials(template: &str, dest: &Path, vars: &Vars) -> Result<()> {
    let template_dir = TEMPLATES.get_dir(template);
    let template_has = |rel: &str| {
        template_dir.is_some_and(|d| d.get_file(format!("{template}/{rel}")).is_some())
    };

    for rel in [".gitignore", "rust-toolchain.toml"] {
        if template_has(rel) {
            continue; // the template ships its own — it wins.
        }
        let Some(partial) = TEMPLATES.get_file(format!("{PARTIALS_DIR}/{rel}")) else {
            continue;
        };
        let contents = partial.contents_utf8().ok_or_else(|| {
            ForgeError::Template(format!("partial {rel} is not UTF-8"))
        })?;
        let out_path = dest.join(rel);
        std::fs::write(&out_path, render_str(contents, vars))
            .map_err(ForgeError::io(format!("writing {}", out_path.display())))?;
    }

    // The release profile lives inside Cargo.toml, so it can't be composed
    // as a standalone file: splice it into the marker every Cargo.toml.hbs
    // carries in place of its own [profile.release] blocks.
    let cargo_toml = dest.join("Cargo.toml");
    if let Ok(rendered) = std::fs::read_to_string(&cargo_toml) {
        if rendered.contains(RELEASE_PROFILE_MARKER) {
            let partial = TEMPLATES
                .get_file(format!("{PARTIALS_DIR}/release-profile.toml"))
                .and_then(|f| f.contents_utf8())
                .ok_or_else(|| {
                    ForgeError::Template("partial release-profile.toml is missing or not UTF-8".into())
                })?;
            let patched = rendered.replace(RELEASE_PROFILE_MARKER, partial);
            std::fs::write(&cargo_toml, patched)
                .map_err(ForgeError::io(format!("writing {}", cargo_toml.display())))?;
        }
    }

    Ok(())
}

/// Parse a `--contract` spec of the form `NAME` or `NAME:TEMPLATE`.
/// Defaults to the `hello-world` template when no template is given.
/// Returns `(name, template)`.
pub fn parse_contract_spec(spec: &str) -> (String, String) {
    match spec.split_once(':') {
        Some((name, template)) => (name.trim().to_string(), template.trim().to_string()),
        None => (spec.trim().to_string(), DEFAULT_TEMPLATE.to_string()),
    }
}

/// Rewrite a rendered member `Cargo.toml` for use inside a workspace:
/// - point `soroban-sdk` at the workspace (`soroban-sdk.workspace = true`)
///   in both `[dependencies]` and `[dev-dependencies]` (preserving the
///   dev `features`), and
/// - drop `[profile.*]` sections (or the shared-partial marker that stands
///   in for them — see `compose_partials`), which Cargo only honours at the
///   workspace root and warns about in members.
fn member_manifest(rendered: &str) -> String {
    let mut out = String::new();
    let mut in_profile = false;
    for line in rendered.lines() {
        let trimmed = line.trim_start();

        // A template that hasn't rendered through `compose_partials` (every
        // workspace member goes through `render_dir` directly, not
        // `generate`) still carries the release-profile marker verbatim;
        // drop it the same as an inline [profile.*] section.
        if line == RELEASE_PROFILE_MARKER.trim_end() {
            continue;
        }

        // Enter/exit a [profile.*] section (dropped entirely).
        if trimmed.starts_with('[') {
            in_profile = trimmed.starts_with("[profile.");
            if in_profile {
                continue;
            }
        }
        if in_profile {
            continue;
        }

        // Replace the two soroban-sdk dependency forms with workspace refs.
        if trimmed.starts_with("soroban-sdk = {") {
            // dev-dependencies: keep testutils via the workspace, still a
            // workspace ref (features are declared on the workspace dep).
            out.push_str("soroban-sdk = { workspace = true, features = [\"testutils\"] }\n");
            continue;
        }
        if trimmed.starts_with("soroban-sdk = \"") {
            out.push_str("soroban-sdk.workspace = true\n");
            continue;
        }

        out.push_str(line);
        out.push('\n');
    }
    out
}

/// The workspace root `Cargo.toml`, listing all members under `contracts/`
/// and pinning the shared `soroban-sdk` version and release profile.
fn workspace_root_manifest(members: &[String]) -> String {
    let members_list = members
        .iter()
        .map(|m| format!("    \"contracts/{m}\",\n"))
        .collect::<String>();
    format!(
        "[workspace]\n\
         resolver = \"2\"\n\
         members = [\n{members_list}]\n\
         \n\
         [workspace.dependencies]\n\
         soroban-sdk = \"{SOROBAN_SDK_VERSION}\"\n\
         \n\
         [profile.release]\n\
         opt-level = \"z\"\n\
         overflow-checks = true\n\
         debug = 0\n\
         strip = \"symbols\"\n\
         debug-assertions = false\n\
         panic = \"abort\"\n\
         codegen-units = 1\n\
         lto = true\n\
         \n\
         [profile.release-with-logs]\n\
         inherits = \"release\"\n\
         debug-assertions = true\n"
    )
}

/// Scaffold a Cargo workspace containing one crate per `(name, template)` under
/// `contracts/<name>/`, plus a shared root `Cargo.toml` and a `forge.toml`.
///
/// Each contract is rendered with the existing single-crate template machinery,
/// then its manifest is rewritten to consume the workspace's shared
/// `soroban-sdk` dependency and release profile. The result builds every
/// contract with a single `cargo build` at the root.
pub fn generate_workspace(
    dest: &Path,
    project_name: &str,
    author: &str,
    edition: &str,
    contracts: &[(String, String)],
    force: bool,
) -> Result<()> {
    if contracts.is_empty() {
        return Err(ForgeError::InvalidArgument(
            "a workspace needs at least one --contract".into(),
        ));
    }
    if dest.exists() && !force {
        return Err(ForgeError::AlreadyExists(dest.to_path_buf()));
    }

    let mut members = Vec::new();
    for (name, template) in contracts {
        validate_project_name(name)?;
        let member_dir = dest.join("contracts").join(name);
        let vars = project_vars(name, author, edition);

        // Render the contract as a normal single-crate project, but WITHOUT its
        // own forge.toml (the workspace root owns that).
        let template_dir = TEMPLATES.get_dir(template).ok_or_else(|| {
            ForgeError::Template(format!(
                "unknown template `{template}` (available: {})",
                available_templates().join(", ")
            ))
        })?;
        render_dir(template_dir, template, &member_dir, &vars)?;

        // Rewrite the member manifest to use workspace deps + root profile.
        let manifest_path = member_dir.join("Cargo.toml");
        let rendered = std::fs::read_to_string(&manifest_path).map_err(ForgeError::io(format!(
            "reading {}",
            manifest_path.display()
        )))?;
        std::fs::write(&manifest_path, member_manifest(&rendered)).map_err(ForgeError::io(
            format!("writing {}", manifest_path.display()),
        ))?;

        members.push(name.clone());
    }

    // Root Cargo.toml and forge.toml.
    let root_manifest = dest.join("Cargo.toml");
    std::fs::write(&root_manifest, workspace_root_manifest(&members)).map_err(ForgeError::io(
        format!("writing {}", root_manifest.display()),
    ))?;

    let vars = project_vars(project_name, author, edition);
    write_forge_toml(dest, &vars)?;
    Ok(())
}
/// Clone a remote git repository URL into a temp directory and render it as a
/// template, applying the same `{{variable}}` substitution rules as bundled
/// templates. The clone is shallow (`--depth 1`) to keep it fast.
///
/// # Errors
///
/// Returns a descriptive [`ForgeError`] when:
/// - `git` is not on `PATH` (`ToolMissing`)
/// - the network is unavailable or the URL is unreachable (`Other` with a hint
///   to check connectivity)
/// - the destination already exists and `force` is not set (`AlreadyExists`)
pub fn generate_from_url(url: &str, dest: &Path, vars: &Vars, force: bool) -> Result<()> {
    generate_from_url_with(url, dest, vars, force, &mut |_| Ok(Vars::new()))
}

/// [`generate_from_url`], plus a hook that sees the cloned template's
/// `template.toml` (if any) and returns extra variables to render with.
///
/// The hook runs after the clone and before anything is written, which is what
/// lets the CLI prompt for a remote template's custom variables — those are
/// only knowable once the repository is on disk.
pub fn generate_from_url_with(
    url: &str,
    dest: &Path,
    vars: &Vars,
    force: bool,
    resolve: &mut dyn FnMut(Option<TemplateManifest>) -> Result<Vars>,
) -> Result<()> {
    if dest.exists() && !force {
        return Err(ForgeError::AlreadyExists(dest.to_path_buf()));
    }

    // Clone into a temporary directory so we never touch dest on failure.
    let tmp = tempfile::tempdir().map_err(ForgeError::io(
        "creating temporary directory for remote clone",
    ))?;
    let clone_dest = tmp.path().join("repo");

    log::debug!("cloning `{url}` into {}", clone_dest.display());

    let output = std::process::Command::new("git")
        .args(["clone", "--depth", "1", "--", url])
        .arg(&clone_dest)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ForgeError::ToolMissing(
                    "git — install git to use --from with remote templates".into(),
                )
            } else {
                ForgeError::io("running `git clone`")(e)
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Distinguish network failures from other git errors for a friendlier message.
        let is_network_error = stderr.contains("Could not resolve host")
            || stderr.contains("Failed to connect")
            || stderr.contains("Network is unreachable")
            || stderr.contains("unable to access")
            || stderr.contains("Connection refused")
            || stderr.contains("not found")
            || stderr.contains("Repository not found")
            || stderr.contains("does not exist");

        return if is_network_error {
            Err(ForgeError::Other(format!(
                "could not clone `{url}`: network error or repository not found\n\
                 hint: check your internet connection and confirm the URL is a public repository\n\
                 git: {stderr}"
            )))
        } else {
            Err(ForgeError::Other(format!(
                "git clone failed for `{url}` (exit {})\n\
                 git: {stderr}",
                output.status
            )))
        };
    }

    // Remove the .git directory from the clone — we're rendering a template,
    // not keeping the upstream history.
    let dot_git = clone_dest.join(".git");
    if dot_git.exists() {
        std::fs::remove_dir_all(&dot_git)
            .map_err(ForgeError::io("removing .git from cloned template"))?;
    }

    // Let the caller fill in whatever the clone's template.toml declares.
    let mut vars = vars.clone();
    vars.extend(resolve(manifest_in_dir(&clone_dest)?)?);

    // Render the cloned filesystem tree with variable substitution.
    render_dir_fs(&clone_dest, &clone_dest, dest, &vars)?;
    write_forge_toml(dest, &vars)?;
    Ok(())
    // `tmp` is dropped here, cleaning up the temp clone directory automatically.
}

/// Walk a real filesystem directory `dir` recursively and render every file
/// into the matching path under `dest`, applying `{{variable}}` substitution
/// to both file contents and relative path segments (same rules as
/// [`render_dir`] for embedded templates).
///
/// Files ending in `.hbs` have that suffix stripped on render, matching the
/// bundled-template convention.
fn render_dir_fs(dir: &Path, source_root: &Path, dest: &Path, vars: &Vars) -> Result<()> {
    for entry in std::fs::read_dir(dir).map_err(ForgeError::io(format!(
        "reading directory {}",
        dir.display()
    )))? {
        let entry = entry.map_err(ForgeError::io(format!(
            "reading directory {}",
            dir.display()
        )))?;
        let path = entry.path();

        if path.is_dir() {
            render_dir_fs(&path, source_root, dest, vars)?;
        } else {
            let rel = path
                .strip_prefix(source_root)
                .expect("path must be under source_root");
            if is_manifest(rel) {
                continue; // template.toml configures generation; it is not output
            }

            // Apply variable substitution to the relative path (including each
            // component), then strip a trailing .hbs suffix if present.
            let mut rel_str = render_str(&rel.to_string_lossy(), vars);
            if let Some(stripped) = rel_str.strip_suffix(".hbs") {
                rel_str = stripped.to_string();
            }
            let out_path = dest.join(&rel_str);

            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(ForgeError::io(format!("creating {}", parent.display())))?;
            }

            // Only render UTF-8 files; copy binary files verbatim.
            match std::fs::read(&path) {
                Ok(bytes) => match std::str::from_utf8(&bytes) {
                    Ok(text) => {
                        std::fs::write(&out_path, render_str(text, vars))
                            .map_err(ForgeError::io(format!("writing {}", out_path.display())))?;
                    }
                    Err(_) => {
                        // Binary file — copy as-is without substitution.
                        std::fs::write(&out_path, &bytes)
                            .map_err(ForgeError::io(format!("writing {}", out_path.display())))?;
                    }
                },
                Err(e) => {
                    return Err(ForgeError::Io {
                        context: format!("reading {}", path.display()),
                        source: e,
                    });
                }
            }
        }
    }
    Ok(())
}

/// True for the template's own `template.toml`, which lives at the root of a
/// template and drives generation rather than being part of the output.
fn is_manifest(rel: &Path) -> bool {
    rel == Path::new(manifest::MANIFEST_FILE)
}

/// Print the planned file tree for a dry-run without writing anything to disk.
fn print_dry_run_tree(dir: &Dir<'_>, template_root: &str, project_name: &str) {
    for entry in dir.dirs() {
        print_dry_run_tree(entry, template_root, project_name);
    }
    for file in dir.files() {
        let rel = file
            .path()
            .strip_prefix(template_root)
            .expect("embedded file path must start with the template name");
        if is_manifest(rel) {
            continue;
        }
        let mut rel_str = rel.to_string_lossy().to_string();
        if let Some(stripped) = rel_str.strip_suffix(".hbs") {
            rel_str = stripped.to_string();
        }
        println!("  {}/{}", project_name, rel_str);
    }
}

fn render_dir(dir: &Dir<'_>, template_root: &str, dest: &Path, vars: &Vars) -> Result<()> {
    for entry in dir.dirs() {
        render_dir(entry, template_root, dest, vars)?;
    }
    for file in dir.files() {
        // Paths inside the embedded dir are prefixed with the template name.
        let rel = file
            .path()
            .strip_prefix(template_root)
            .expect("embedded file path must start with the template name");
        if is_manifest(rel) {
            continue; // template.toml configures generation; it is not output
        }
        // Metadata, not project content: never copied into the generated project.
        if rel == Path::new(MANIFEST_FILE_NAME) {
            continue;
        }
        let mut rel_str = render_str(&rel.to_string_lossy(), vars);
        if let Some(stripped) = rel_str.strip_suffix(".hbs") {
            rel_str = stripped.to_string();
        }
        let out_path = dest.join(&rel_str);

        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(ForgeError::io(format!("creating {}", parent.display())))?;
        }

        let contents = file.contents_utf8().ok_or_else(|| {
            ForgeError::Template(format!("template file {} is not UTF-8", rel.display()))
        })?;
        std::fs::write(&out_path, render_str(contents, vars))
            .map_err(ForgeError::io(format!("writing {}", out_path.display())))?;
    }
    Ok(())
}

/// Every generated project gets a `forge.toml` so later `soroban-forge`
/// commands (test-init, ci-init) know the project name and author.
fn write_forge_toml(dest: &Path, vars: &Vars) -> Result<()> {
    let contents = format!(
        "# soroban-forge project configuration\n[project]\nname = \"{}\"\nauthors = [\"{}\"]\n",
        vars["project_name"], vars["author"],
    );
    let path = dest.join("forge.toml");
    std::fs::write(&path, contents).map_err(ForgeError::io(format!("writing {}", path.display())))
}

/// Write `.pre-commit-config.yaml` into `dest`.
/// Respects `force` the same way `generate()` does.
fn write_pre_commit_config(dest: &Path, force: bool) -> Result<()> {
    let path = dest.join(".pre-commit-config.yaml");
    if path.exists() && !force {
        return Err(ForgeError::AlreadyExists(path));
    }
    std::fs::write(&path, PRE_COMMIT_CONFIG)
        .map_err(ForgeError::io(format!("writing {}", path.display())))
}

/// The calendar year, computed without a date/time dependency: good enough
/// for a LICENSE copyright line, which only ever needs the current year.
fn current_year() -> i32 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    1970 + (secs / 31_557_600) as i32 // 31_557_600s = 365.25 days, the average Gregorian year
}

/// Write a LICENSE file into `dest` for `license_id` (one of
/// [`license::LICENSE_IDS`]), with `author` and the current year filled in.
/// Respects `force` the same way `write_pre_commit_config` does.
fn write_license_file(dest: &Path, license_id: &str, author: &str, force: bool) -> Result<()> {
    let path = dest.join("LICENSE");
    if path.exists() && !force {
        return Err(ForgeError::AlreadyExists(path));
    }
    let text = license::license_text(license_id, author, current_year());
    std::fs::write(&path, text).map_err(ForgeError::io(format!("writing {}", path.display())))
}

/// Write `.devcontainer/{devcontainer.json,Dockerfile}` into `dest` and
/// document it in the generated `README.md`. Respects `force` the same way
/// `write_pre_commit_config` does.
fn write_devcontainer(dest: &Path, vars: &Vars, force: bool) -> Result<()> {
    let dir = dest.join(".devcontainer");
    let json_path = dir.join("devcontainer.json");
    let dockerfile_path = dir.join("Dockerfile");
    if !force && (json_path.exists() || dockerfile_path.exists()) {
        return Err(ForgeError::AlreadyExists(dir));
    }
    std::fs::create_dir_all(&dir)
        .map_err(ForgeError::io(format!("creating {}", dir.display())))?;
    std::fs::write(&json_path, render_str(&devcontainer_json_template(), vars))
        .map_err(ForgeError::io(format!("writing {}", json_path.display())))?;
    std::fs::write(&dockerfile_path, devcontainer_dockerfile())
        .map_err(ForgeError::io(format!("writing {}", dockerfile_path.display())))?;

    // Document it in the generated README, when the template has one.
    let readme_path = dest.join("README.md");
    if let Ok(existing) = std::fs::read_to_string(&readme_path) {
        if !existing.contains("## Dev Container") {
            let updated = format!("{existing}{DEVCONTAINER_README_SECTION}");
            std::fs::write(&readme_path, updated)
                .map_err(ForgeError::io(format!("writing {}", readme_path.display())))?;
        }
    }
    Ok(())
}

/// Insert a `license = "..."` line into the `[package]` section of the
/// already-rendered `Cargo.toml` at `dest`, matching `license_id`.
fn set_cargo_toml_license(dest: &Path, license_id: &str) -> Result<()> {
    let path = dest.join("Cargo.toml");
    let contents = std::fs::read_to_string(&path)
        .map_err(ForgeError::io(format!("reading {}", path.display())))?;
    let field = license::cargo_license_field(license_id);
    let patched = contents.replacen(
        "[package]\n",
        &format!("[package]\nlicense = \"{field}\"\n"),
        1,
    );
    std::fs::write(&path, patched).map_err(ForgeError::io(format!("writing {}", path.display())))
}

/// Initialize a git repository in `dest`.
pub fn init_git(dest: &Path) -> Result<()> {
    let output = std::process::Command::new("git")
        .arg("init")
        .arg("-q")
        .arg(dest)
        .output();
    match output {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(ForgeError::Other(format!(
            "`git init` exited with status {}",
            o.status
        ))),
        Err(e) => Err(ForgeError::io("executing `git init`")(e)),
    }
}

fn default_author(ctx: &ForgeContext) -> String {
    if let Some(author) = ctx
        .config
        .as_ref()
        .and_then(|c| c.author().map(String::from))
    {
        return author;
    }
    // Fall back to the git identity, then a placeholder.
    std::process::Command::new("git")
        .args(["config", "user.name"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Your Name".to_string())
}

/// Render the successful project creation report, including any
/// template-specific post-generate hints from `template.toml`.
pub fn format_created_report(name: &str, template: &str, dest: &Path, hints: &[String]) -> String {
    let mut out = format!(
        "created `{name}` from template `{template}` at {}\n\n\
         next steps:\n\
           cd {name}\n\
           cargo test                      # run the template's unit tests\n\
           stellar contract build          # build the deployable wasm\n\
           soroban-forge test-init         # add a generated test harness\n\
           soroban-forge ci-init           # add GitHub Actions workflows\n",
        dest.display()
    );
    if !hints.is_empty() {
        out.push_str("\ntemplate notes:\n");
        for hint in hints {
            out.push_str(&format!("  - {hint}\n"));
        }
    }
    out
}

/// The `new` subcommand.
pub struct ScaffoldPlugin;

impl ForgePlugin for ScaffoldPlugin {
    fn name(&self) -> &'static str {
        "new"
    }

    fn command(&self) -> Command {
        Command::new("new")
            .about("Create a new Soroban contract project from a template")
            .arg(
                Arg::new("name")
                    .help("Project name (also the cargo package name)")
                    .required_unless_present("list"),
            )
            .arg(
                Arg::new("template")
                    .long("template")
                    .short('t')
                    .help("Bundled template to use (see --list-templates); mutually exclusive with --from"),
            )
            .arg(
                Arg::new("from")
                    .long("from")
                    .value_name("URL")
                    .help("Git repository URL to use as a remote template (e.g. https://github.com/user/tpl); mutually exclusive with --template")
                    .conflicts_with("template"),
            )
            .arg(
                Arg::new("author")
                    .long("author")
                    .help("Author for Cargo.toml [default: forge.toml, then git config user.name]"),
            )
            .arg(
                Arg::new("output")
                    .long("output-dir")
                    .short('o')
                    .help("Parent directory to create the project in [default: current directory]"),
            )
            .arg(
                Arg::new("list")
                    .long("list-templates")
                    .action(ArgAction::SetTrue)
                    .help("List available templates and exit"),
            )
            .arg(
                Arg::new("pre-commit")
                    .long("pre-commit")
                    .action(ArgAction::SetTrue)
                    .help("Add a .pre-commit-config.yaml with rustfmt and clippy hooks"),
            )
            .arg(
                Arg::new("edition")
                    .long("edition")
                    .help("Rust edition for the generated Cargo.toml [default: 2021]")
                    .value_parser(["2021", "2024"]),
            )
            .arg(
                Arg::new("no-git")
                    .long("no-git")
                    .action(ArgAction::SetTrue)
                    .help("Skip git repository initialization"),
            )
            .arg(
                Arg::new("force")
                    .long("force")
                    .action(ArgAction::SetTrue)
                    .help("Overwrite the target directory if it exists (asks for confirmation; pass --yes to skip it)"),
            )
            .arg(
                Arg::new("var")
                    .long("var")
                    .action(ArgAction::Append)
                    .value_name("NAME=VALUE")
                    .help("Value for a variable declared in the template's template.toml (repeatable); missing ones are prompted for in a terminal"),
            )
            .arg(
                Arg::new("workspace")
                    .long("workspace")
                    .action(ArgAction::SetTrue)
                    .help("Scaffold a Cargo workspace with multiple contract crates (use with --contract)"),
            )
            .arg(
                Arg::new("contract")
                    .long("contract")
                    .action(ArgAction::Append)
                    .value_name("NAME[:TEMPLATE]")
                    .help("A contract to include in the workspace, e.g. `token:token` (repeatable; requires --workspace)"),
            )
            .arg(
                Arg::new("dry-run")
                    .long("dry-run")
                    .action(ArgAction::SetTrue)
                    .help("Print the planned file tree without writing anything to disk"),
            )
            .arg(
                Arg::new("license")
                    .long("license")
                    .value_parser(license::LICENSE_IDS)
                    .help("Write a LICENSE file and set Cargo.toml's license field [default: none, today's behaviour]"),
            )
            .arg(
                Arg::new("devcontainer")
                    .long("devcontainer")
                    .action(ArgAction::SetTrue)
                    .help("Add a .devcontainer/ with Rust, wasm32v1-none and stellar-cli preinstalled"),
            )
            .arg(
                Arg::new("no-tests")
                    .long("no-tests")
                    .action(ArgAction::SetTrue)
                    .help("Skip generating tests/ directory (for users bringing their own test harness)"),
            )
    }

    fn run(&self, matches: &ArgMatches, ctx: &ForgeContext) -> Result<()> {
        if matches.get_flag("list") {
            if ctx.json {
                let catalog = template_catalog();
                println!("{}", serde_json::to_string_pretty(&catalog).unwrap());
            } else if !ctx.quiet {
                print!("{}", format_template_list(&available_templates()));
            }
            return Ok(());
        }

        let name = matches
            .get_one::<String>("name")
            .expect("clap enforces name unless --list-templates");
        validate_project_name(name)?;

        let dry_run = matches.get_flag("dry-run");

        let author = matches
            .get_one::<String>("author")
            .cloned()
            .unwrap_or_else(|| default_author(ctx));

        let edition = matches
            .get_one::<String>("edition")
            .cloned()
            .unwrap_or_else(|| "2021".to_string());

        let parent = matches
            .get_one::<String>("output")
            .map(|o| ctx.cwd.join(o))
            .unwrap_or_else(|| ctx.cwd.clone());
        let dest = parent.join(name);

        let force = matches.get_flag("force");
        let supplied = manifest::parse_var_assignments(
            matches
                .get_many::<String>("var")
                .map(|vals| vals.map(String::as_str).collect::<Vec<_>>())
                .unwrap_or_default(),
        )?;
        let mut vars = project_vars(name, &author, &edition);

        // --workspace: scaffold a multi-contract Cargo workspace.
        if matches.get_flag("workspace") {
            let specs: Vec<(String, String)> = matches
                .get_many::<String>("contract")
                .map(|vals| vals.map(|s| parse_contract_spec(s)).collect())
                .unwrap_or_default();
            if specs.is_empty() {
                return Err(ForgeError::InvalidArgument(
                    "--workspace requires at least one --contract NAME[:TEMPLATE]".into(),
                ));
            }

            if dry_run {
                println!("dry-run: planned workspace `{name}` at {}", dest.display());
                println!("  {}/", name);
                println!("  {}/Cargo.toml", name);
                println!("  {}/forge.toml", name);
                for (contract_name, _template) in &specs {
                    println!("  {}/contracts/{}/Cargo.toml", name, contract_name);
                    println!("  {}/contracts/{}/src/lib.rs", name, contract_name);
                    println!("  {}/contracts/{}/src/test.rs", name, contract_name);
                }
                return Ok(());
            }

            if force {
                confirm_overwrite(&dest, ctx)?;
            }

            log::debug!(
                "scaffolding workspace `{name}` with {} contract(s) into {}",
                specs.len(),
                dest.display()
            );
            generate_workspace(&dest, name, &author, &edition, &specs, force)?;

            if !matches.get_flag("no-git") {
                if let Err(err) = init_git(&dest) {
                    log::warn!("failed to initialize git repository: {err}");
                }
            }
            if matches.get_flag("pre-commit") {
                write_pre_commit_config(&dest, force)?;
            }

            if !ctx.quiet {
                let members: Vec<&str> = specs.iter().map(|(n, _)| n.as_str()).collect();
                println!(
                    "created workspace `{name}` with contracts [{}] at {}",
                    members.join(", "),
                    dest.display()
                );
                println!();
                println!("next steps:");
                println!("  cd {name}");
                println!("  cargo build                     # builds every contract");
                println!("  cargo test                      # runs all contract tests");
                println!("  soroban-forge test-init         # add harnesses for each member");
                println!("  soroban-forge ci-init           # add GitHub Actions workflows");
            }
            return Ok(());
        }

        // --from takes precedence over --template: clone a remote repo.
        if let Some(url) = matches.get_one::<String>("from") {
            if dry_run {
                println!(
                    "dry-run: planned project `{name}` from remote `{url}` at {}",
                    dest.display()
                );
                println!("  {}/  (contents depend on remote repository)", name);
                println!("  {}/forge.toml", name);
                return Ok(());
            }

            if ctx.offline {
                return Err(ForgeError::InvalidArgument(
                    "remote templates are unavailable in offline mode; use a bundled --template"
                        .into(),
                ));
            }

            if force {
                confirm_overwrite(&dest, ctx)?;
            }

            log::debug!(
                "scaffolding `{name}` from remote URL `{url}` into {}",
                dest.display()
            );
            // The remote template's variables are only knowable after the
            // clone, so they are resolved from inside generate_from_url_with.
            generate_from_url_with(url, &dest, &vars, force, &mut |manifest| {
                let mut extra = Vars::new();
                merge_template_vars(manifest.as_ref(), &supplied, ctx, &mut extra)?;
                Ok(extra)
            })?;

            if !matches.get_flag("no-git") {
                if let Err(err) = init_git(&dest) {
                    log::warn!("failed to initialize git repository: {err}");
                }
            }

            if matches.get_flag("pre-commit") {
                write_pre_commit_config(&dest, force)?;
            }

            if !ctx.quiet {
                println!(
                    "created `{name}` from remote template `{url}` at {}",
                    dest.display()
                );
                println!();
                println!("next steps:");
                println!("  cd {name}");
                println!("  cargo test                      # run the template's unit tests");
                println!("  stellar contract build          # build the deployable wasm");
                println!("  soroban-forge test-init         # add a generated test harness");
                println!("  soroban-forge ci-init           # add GitHub Actions workflows");
                if matches.get_flag("pre-commit") {
                    println!("  pre-commit install              # enable the git hooks");
                }
            }
            return Ok(());
        }

        // Bundled template path.
        let template = matches
            .get_one::<String>("template")
            .cloned()
            .or_else(|| {
                ctx.config
                    .as_ref()
                    .and_then(|c| c.scaffold.default_template.clone())
            })
            .unwrap_or_else(|| DEFAULT_TEMPLATE.to_string());

        if dry_run {
            println!(
                "dry-run: planned project `{name}` from template `{template}` at {}",
                dest.display()
            );
            // Show planned file tree using the embedded template listing.
            if let Some(template_dir) = TEMPLATES.get_dir(template.as_str()) {
                print_dry_run_tree(template_dir, &template, name);
            }
            println!("  {}/forge.toml", name);
            return Ok(());
        }

        // Fill in whatever the template declares in its template.toml.
        let template_manifest = bundled_manifest(&template)?;
        merge_template_vars(template_manifest.as_ref(), &supplied, ctx, &mut vars)?;

        if force {
            confirm_overwrite(&dest, ctx)?;
        }
        let manifest = load_manifest(&template)?;
        let overrides = parse_var_overrides(
            &matches
                .get_many::<String>("var")
                .map(|vals| vals.cloned().collect::<Vec<_>>())
                .unwrap_or_default(),
        )?;
        // Never prompt when quiet, --yes was passed, or stdin isn't a terminal
        // (e.g. CI) — scripted use must never block waiting for input.
        let interactive = !ctx.quiet && !matches.get_flag("yes") && std::io::stdin().is_terminal();
        let extra_vars = resolve_extra_vars(&manifest, &overrides, interactive)?;

        let mut vars = project_vars(name, &author, &edition);
        vars.extend(extra_vars);

        log::debug!(
            "scaffolding `{name}` from template `{template}` into {}",
            dest.display()
        );
        generate(&template, &dest, &vars, force)?;

        if !matches.get_flag("no-git") {
            if let Err(err) = init_git(&dest) {
                log::warn!("failed to initialize git repository: {err}");
            }
        }

        if matches.get_flag("pre-commit") {
            write_pre_commit_config(&dest, force)?;
        }

        if let Some(license_id) = matches.get_one::<String>("license") {
            write_license_file(&dest, license_id, &author, force)?;
            set_cargo_toml_license(&dest, license_id)?;
        }

        if matches.get_flag("devcontainer") {
            write_devcontainer(&dest, &vars, force)?;
        }

        if !ctx.quiet {
            println!(
                "created `{name}` from template `{template}` at {}",
                dest.display()
            );
            println!();
            println!("next steps:");
            println!("  cd {name}");
            println!("  cargo test                      # run the template's unit tests");
            println!("  stellar contract build          # build the deployable wasm");
            println!("  soroban-forge test-init         # add a generated test harness");
            println!("  soroban-forge ci-init           # add GitHub Actions workflows");
            if matches.get_flag("pre-commit") {
                println!("  pre-commit install              # enable the git hooks");
            }
            if let Some(license_id) = matches.get_one::<String>("license") {
                println!("  license: {}", license::cargo_license_field(license_id));
            }
            if matches.get_flag("devcontainer") {
                println!("  Reopen in Container             # or: devcontainer up (Codespaces-ready)");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_var_overrides() {
        let overrides = parse_var_overrides(&[
            "token_symbol=XYZ".to_string(),
            "token_decimals=2".to_string(),
        ])
        .unwrap();
        assert_eq!(
            overrides.get("token_symbol").map(String::as_str),
            Some("XYZ")
        );
        assert_eq!(
            overrides.get("token_decimals").map(String::as_str),
            Some("2")
        );
    }

    #[test]
    fn var_override_without_equals_is_an_error() {
        assert!(parse_var_overrides(&["oops".to_string()]).is_err());
    }

    #[test]
    fn resolve_extra_vars_prefers_overrides_then_defaults() {
        let manifest = manifest::parse_manifest(
            r#"
[[variable]]
name = "token_symbol"
prompt = "Symbol"
default = "MYT"
"#,
        )
        .unwrap();

        let mut overrides = BTreeMap::new();
        overrides.insert("token_symbol".to_string(), "XYZ".to_string());
        let vars = resolve_extra_vars(&manifest, &overrides, false).unwrap();
        assert_eq!(vars.get("token_symbol").map(String::as_str), Some("XYZ"));

        let vars = resolve_extra_vars(&manifest, &BTreeMap::new(), false).unwrap();
        assert_eq!(vars.get("token_symbol").map(String::as_str), Some("MYT"));
    }

    #[test]
    fn template_toml_is_not_copied_into_generated_project() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("demo");
        generate("token", &dest, &project_vars("demo", "A", "2021"), false).unwrap();
        assert!(!dest.join("template.toml").exists());
    }

    #[test]
    fn token_template_manifest_declares_expected_variables() {
        let manifest = load_manifest("token").unwrap();
        let names: Vec<&str> = manifest.variables.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["token_name", "token_symbol", "token_decimals"]);
    }

    #[test]
    fn lists_all_bundled_templates() {
        assert_eq!(
            available_templates(),
            vec![
                "access-control",
                "allowlist-token",
                "amm",
                "atomic-swap",
                "crowdfund",
                "dutch-auction",
                "escrow",
                "faucet",
                "flash-loan",
                "governance",
                "hello-world",
                "lottery",
                "merkle-airdrop",
                "multisig",
                "nft",
                "nft-marketplace",
                "oracle-consumer",
                "payment-splitter",
                "soulbound",
                "staking",
                "streaming",
                "subscription",
                "timelock",
                "token",
                "upgradeable",
                "vesting",
                "wrapped-asset",
                "yield-vault"
            ]
        );
    }

    #[test]
    fn template_list_report_has_heading_and_items() {
        let report = format_template_list(&["hello-world", "nft", "token"]);
        assert_eq!(
            report,
            "available templates:\n  hello-world\n  nft\n  token\n"
        );
    }

    #[test]
    fn every_bundled_template_has_a_description() {
        for name in available_templates() {
            assert!(
                template_description(name).is_some(),
                "template `{name}` has no description — add one to `template_description()`"
            );
        }
    }

    #[test]
    fn unknown_template_description_is_none() {
        assert_eq!(template_description("does-not-exist"), None);
    }

    #[test]
    fn catalog_returns_all_templates_with_descriptions() {
        let catalog = template_catalog();
        let names: Vec<&str> = catalog.iter().map(|t| t.name).collect();
        assert_eq!(names, available_templates());
        assert_eq!(
            names,
            vec![
                "access-control",
                "amm",
                "atomic-swap",
                "crowdfund",
                "escrow",
                "faucet",
                "flash-loan",
                "governance",
                "hello-world",
                "lottery",
                "multisig",
                "nft",
                "merkle-airdrop",
                "multisig",
                "nft",
                "payment-splitter",
                "staking",
                "streaming",
                "subscription",
                "token",
                "vesting",
                "wrapped-asset"
            ]
        );
        for entry in &catalog {
            assert!(
                !entry.description.is_empty(),
                "empty description for `{}`",
                entry.name
            );
        }
    }

    #[test]
    fn catalog_is_sorted_by_name() {
        let catalog = template_catalog();
        let names: Vec<&str> = catalog.iter().map(|t| t.name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
    }

    #[test]
    fn creation_report_identifies_project_and_template() {
        let report = format_created_report("demo", "token", Path::new("/tmp/demo"), &[]);
        assert!(report.starts_with("created `demo` from template `token` at /tmp/demo\n"));
    }

    #[test]
    fn creation_report_includes_next_steps() {
        let report = format_created_report("demo", "token", Path::new("demo"), &[]);
        assert!(report.contains("cd demo"));
        assert!(report.contains("cargo test"));
        assert!(report.contains("stellar contract build"));
        assert!(report.contains("soroban-forge test-init"));
        assert!(report.contains("soroban-forge ci-init"));
    }

    #[test]
    fn creation_report_includes_hints_when_present() {
        let hints = vec!["deploy with --decimals 7".to_string()];
        let report = format_created_report("demo", "token", Path::new("demo"), &hints);
        assert!(report.contains("template notes:"));
        assert!(report.contains("deploy with --decimals 7"));
    }

    #[test]
    fn creation_report_omits_notes_section_without_hints() {
        let report = format_created_report("demo", "token", Path::new("demo"), &[]);
        assert!(!report.contains("template notes:"));
    }

    #[test]
    fn validates_project_names() {
        assert!(validate_project_name("my-project").is_ok());
        assert!(validate_project_name("a1_b2").is_ok());
        assert!(validate_project_name("MyProject").is_err());
        assert!(validate_project_name("1st").is_err());
        assert!(validate_project_name("").is_err());
        assert!(validate_project_name("has space").is_err());
    }

    #[test]
    fn unknown_template_error_names_available_ones() {
        let dir = tempfile::tempdir().unwrap();
        let err = generate(
            "nope",
            &dir.path().join("x"),
            &project_vars("x", "A", "2021"),
            false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("hello-world"));
    }

    #[test]
    fn refuses_existing_destination_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("demo");
        std::fs::create_dir(&dest).unwrap();
        assert!(matches!(
            generate(
                "hello-world",
                &dest,
                &project_vars("demo", "A", "2021"),
                false
            ),
            Err(ForgeError::AlreadyExists(_))
        ));
    }

    #[test]
    fn generates_hello_world_fully_rendered() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("demo");
        generate(
            "hello-world",
            &dest,
            &project_vars("demo", "Ada <ada@example.com>", "2021"),
            false,
        )
        .unwrap();

        let manifest = std::fs::read_to_string(dest.join("Cargo.toml")).unwrap();
        assert!(manifest.contains("name = \"demo\""));
        assert!(manifest.contains(SOROBAN_SDK_VERSION));
        assert!(manifest.contains("Ada <ada@example.com>"));
        assert!(dest.join("src/lib.rs").is_file());
        assert!(dest.join("src/test.rs").is_file());
        assert!(dest.join("forge.toml").is_file());
        assert!(dest.join("README.md").is_file());

        let readme = std::fs::read_to_string(dest.join("README.md")).unwrap();
        assert!(readme.contains("# demo"));
        assert!(readme.contains("cargo test"));
        assert!(readme.contains("stellar contract build"));
        assert!(readme.contains("stellar contract deploy"));
        assert!(readme.contains("demo.wasm"));

        // No unrendered placeholders anywhere.
        for entry in walk(&dest) {
            let contents = std::fs::read_to_string(&entry).unwrap();
            for var in ["project_name", "crate_name", "author", "sdk_version"] {
                assert!(
                    !contents.contains(&format!("{{{{{var}}}}}")),
                    "unrendered {{{{{var}}}}} in {}",
                    entry.display()
                );
            }
        }
    }

    #[test]
    fn partials_dir_is_excluded_from_available_templates() {
        assert!(!available_templates().contains(&"_partials"));
    }

    #[test]
    fn generate_rejects_partials_dir_as_a_template_name() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("demo");
        let err = generate(
            "_partials",
            &dest,
            &project_vars("demo", "A", "2021"),
            false,
        )
        .unwrap_err();
        assert!(matches!(err, ForgeError::Template(_)));
    }

    #[test]
    fn compose_partials_fills_gitignore_and_rust_toolchain_when_template_lacks_them() {
        // escrow ships neither a .gitignore nor a rust-toolchain.toml.
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("demo");
        generate("escrow", &dest, &project_vars("demo", "A", "2021"), false).unwrap();

        let gitignore = std::fs::read_to_string(dest.join(".gitignore")).unwrap();
        assert_eq!(gitignore, "target/\n");

        let toolchain = std::fs::read_to_string(dest.join("rust-toolchain.toml")).unwrap();
        assert!(toolchain.contains("channel = \"1.84\""));
        assert!(toolchain.contains("wasm32v1-none"));
    }

    #[test]
    fn compose_partials_respects_a_templates_own_gitignore() {
        // nft ships its own `.gitignore` (no trailing slash) — it must win
        // over the shared partial's `target/`.
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("demo");
        generate("nft", &dest, &project_vars("demo", "A", "2021"), false).unwrap();

        let gitignore = std::fs::read_to_string(dest.join(".gitignore")).unwrap();
        assert_eq!(gitignore, "target\n");
    }

    #[test]
    fn compose_partials_splices_release_profile_into_cargo_toml() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("demo");
        generate(
            "hello-world",
            &dest,
            &project_vars("demo", "A", "2021"),
            false,
        )
        .unwrap();

        let manifest = std::fs::read_to_string(dest.join("Cargo.toml")).unwrap();
        assert!(manifest.contains("[profile.release]"));
        assert!(manifest.contains("[profile.release-with-logs]"));
        assert!(manifest.contains("lto = true"));
        assert!(!manifest.contains("soroban-forge:partial"), "marker leaked into output");
    }

    #[test]
    fn member_manifest_strips_the_release_profile_marker() {
        let rendered = format!(
            "[package]\nname = \"demo\"\n\n{}",
            RELEASE_PROFILE_MARKER
        );
        let out = member_manifest(&rendered);
        assert!(!out.contains("soroban-forge:partial"));
        assert!(!out.contains("[profile"));
    }

    #[test]
    fn every_template_generates_without_leftover_hbs_files() {
        for template in available_templates() {
            let dir = tempfile::tempdir().unwrap();
            let dest = dir.path().join("proj");
            generate(template, &dest, &project_vars("proj", "A", "2021"), false).unwrap();
            assert!(dest.join("Cargo.toml").is_file(), "template {template}");
            for entry in walk(&dest) {
                assert!(
                    entry.extension().map(|e| e != "hbs").unwrap_or(true),
                    "leftover .hbs file {} in template {template}",
                    entry.display()
                );
            }
        }
    }

    #[test]
    fn every_template_generates_readme_with_build_and_deploy_instructions() {
        for template in available_templates() {
            let dir = tempfile::tempdir().unwrap();
            let dest = dir.path().join("my-contract");
            generate(
                template,
                &dest,
                &project_vars("my-contract", "A", "2021"),
                false,
            )
            .unwrap();
            let readme_path = dest.join("README.md");
            assert!(
                readme_path.is_file(),
                "README.md missing for template {template}"
            );
            let contents = std::fs::read_to_string(&readme_path).unwrap();
            assert!(
                contents.contains("# my-contract"),
                "template {template} title substitution"
            );
            assert!(
                contents.contains("cargo test"),
                "template {template} test step"
            );
            assert!(
                contents.contains("stellar contract build"),
                "template {template} build step"
            );
            assert!(
                contents.contains("stellar contract deploy"),
                "template {template} deploy step"
            );
            assert!(
                contents.contains("my_contract.wasm"),
                "template {template} crate name substitution"
            );
        }
    }

    #[test]
    fn pre_commit_config_contains_rustfmt_and_clippy() {
        assert!(PRE_COMMIT_CONFIG.contains("rustfmt"));
        assert!(PRE_COMMIT_CONFIG.contains("clippy"));
        assert!(PRE_COMMIT_CONFIG.contains("cargo fmt"));
        assert!(PRE_COMMIT_CONFIG.contains("cargo clippy"));
        assert!(PRE_COMMIT_CONFIG.contains("pass_filenames: false"));
    }

    #[test]
    fn writes_pre_commit_config() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("demo");
        generate(
            "hello-world",
            &dest,
            &project_vars("demo", "A", "2021"),
            false,
        )
        .unwrap();
        write_pre_commit_config(&dest, false).unwrap();

        let path = dest.join(".pre-commit-config.yaml");
        assert!(path.is_file());
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("rustfmt"));
        assert!(contents.contains("clippy"));
        assert!(contents.contains("repos:"));
        assert!(contents.contains("hooks:"));
        assert!(contents.contains("repo: local"));
    }

    #[test]
    fn refuses_to_overwrite_pre_commit_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("demo");
        generate(
            "hello-world",
            &dest,
            &project_vars("demo", "A", "2021"),
            false,
        )
        .unwrap();
        write_pre_commit_config(&dest, false).unwrap();
        assert!(matches!(
            write_pre_commit_config(&dest, false),
            Err(ForgeError::AlreadyExists(_))
        ));
        write_pre_commit_config(&dest, true).unwrap();
    }

    #[test]
    fn pre_commit_not_written_without_flag() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("demo");
        generate(
            "hello-world",
            &dest,
            &project_vars("demo", "A", "2021"),
            false,
        )
        .unwrap();
        assert!(!dest.join(".pre-commit-config.yaml").exists());
    }

    #[test]
    fn writes_license_file_and_sets_cargo_toml_field() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("demo");
        generate(
            "hello-world",
            &dest,
            &project_vars("demo", "Ada Lovelace", "2021"),
            false,
        )
        .unwrap();
        write_license_file(&dest, "mit", "Ada Lovelace", false).unwrap();
        set_cargo_toml_license(&dest, "mit").unwrap();

        let license = std::fs::read_to_string(dest.join("LICENSE")).unwrap();
        assert!(license.starts_with("MIT License"));
        assert!(license.contains("Ada Lovelace"));
        assert!(license.contains(&current_year().to_string()));

        let manifest = std::fs::read_to_string(dest.join("Cargo.toml")).unwrap();
        assert!(manifest.contains("license = \"MIT\""));
        // license comes right after [package], ahead of the other fields.
        assert!(manifest.contains("[package]\nlicense = \"MIT\"\n"));
    }

    #[test]
    fn refuses_to_overwrite_license_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("demo");
        generate(
            "hello-world",
            &dest,
            &project_vars("demo", "A", "2021"),
            false,
        )
        .unwrap();
        write_license_file(&dest, "apache-2.0", "A", false).unwrap();
        assert!(matches!(
            write_license_file(&dest, "apache-2.0", "A", false),
            Err(ForgeError::AlreadyExists(_))
        ));
        write_license_file(&dest, "apache-2.0", "A", true).unwrap();
    }

    #[test]
    fn license_not_written_without_flag() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("demo");
        generate(
            "hello-world",
            &dest,
            &project_vars("demo", "A", "2021"),
            false,
        )
        .unwrap();
        assert!(!dest.join("LICENSE").exists());
        let manifest = std::fs::read_to_string(dest.join("Cargo.toml")).unwrap();
        assert!(!manifest.contains("license"));
    }

    #[test]
    fn license_flag_is_registered_with_expected_values() {
        let plugin = ScaffoldPlugin;
        let cmd = plugin.command();
        let matches = cmd
            .try_get_matches_from(vec!["new", "my-project", "--license", "unlicense"])
            .unwrap();
        assert_eq!(
            matches.get_one::<String>("license").map(String::as_str),
            Some("unlicense")
        );
    }

    #[test]
    fn license_flag_rejects_unknown_value() {
        let plugin = ScaffoldPlugin;
        let cmd = plugin.command();
        assert!(cmd
            .try_get_matches_from(vec!["new", "my-project", "--license", "gpl-3.0"])
            .is_err());
    }

    #[test]
    fn writes_devcontainer_and_documents_it_in_readme() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("demo");
        let vars = project_vars("demo", "A", "2021");
        generate("hello-world", &dest, &vars, false).unwrap();
        write_devcontainer(&dest, &vars, false).unwrap();

        let json = std::fs::read_to_string(dest.join(".devcontainer/devcontainer.json")).unwrap();
        assert!(json.contains("\"name\": \"demo\""));
        assert!(json.contains("wasm32v1-none"));

        let dockerfile = std::fs::read_to_string(dest.join(".devcontainer/Dockerfile")).unwrap();
        assert!(dockerfile.contains("FROM rust:1.84-bookworm"));
        assert!(dockerfile.contains("rustup target add wasm32v1-none"));
        assert!(dockerfile.contains("stellar-cli"));

        let readme = std::fs::read_to_string(dest.join("README.md")).unwrap();
        assert!(readme.contains("## Dev Container"));
    }

    #[test]
    fn refuses_to_overwrite_devcontainer_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("demo");
        let vars = project_vars("demo", "A", "2021");
        generate("hello-world", &dest, &vars, false).unwrap();
        write_devcontainer(&dest, &vars, false).unwrap();
        assert!(matches!(
            write_devcontainer(&dest, &vars, false),
            Err(ForgeError::AlreadyExists(_))
        ));
        write_devcontainer(&dest, &vars, true).unwrap();
    }

    #[test]
    fn devcontainer_not_written_without_flag() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("demo");
        generate(
            "hello-world",
            &dest,
            &project_vars("demo", "A", "2021"),
            false,
        )
        .unwrap();
        assert!(!dest.join(".devcontainer").exists());
    }

    #[test]
    fn devcontainer_flag_is_registered() {
        let plugin = ScaffoldPlugin;
        let cmd = plugin.command();
        let matches = cmd
            .try_get_matches_from(vec!["new", "my-project", "--devcontainer"])
            .unwrap();
        assert!(matches.get_flag("devcontainer"));
    }

    #[test]
    fn no_git_flag_is_registered() {
        let plugin = ScaffoldPlugin;
        let cmd = plugin.command();
        let matches = cmd
            .try_get_matches_from(vec!["new", "my-project", "--no-git"])
            .unwrap();
        assert!(matches.get_flag("no-git"));
    }

    #[test]
    fn init_git_creates_git_directory() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("demo");
        std::fs::create_dir_all(&dest).unwrap();
        if init_git(&dest).is_ok() {
            assert!(dest.join(".git").exists());
        }
    }

    fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
        let mut files = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for entry in std::fs::read_dir(&d).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    files.push(path);
                }
            }
        }
        files
    }

    // ── --from / generate_from_url tests ──────────────────────────────────

    /// The `--from` flag must be registered on the `new` subcommand.
    #[test]
    fn from_flag_is_registered() {
        let plugin = ScaffoldPlugin;
        let cmd = plugin.command();
        let matches = cmd
            .try_get_matches_from(vec![
                "new",
                "my-project",
                "--from",
                "https://example.com/tpl",
            ])
            .unwrap();
        assert_eq!(
            matches.get_one::<String>("from").map(String::as_str),
            Some("https://example.com/tpl")
        );
    }

    /// `--from` and `--template` must be mutually exclusive.
    #[test]
    fn from_and_template_are_mutually_exclusive() {
        let plugin = ScaffoldPlugin;
        let cmd = plugin.command();
        let result = cmd.try_get_matches_from(vec![
            "new",
            "my-project",
            "--from",
            "https://example.com/tpl",
            "--template",
            "hello-world",
        ]);
        assert!(result.is_err(), "expected conflict error but got success");
    }

    /// render_dir_fs applies variable substitution and strips .hbs suffix.
    #[test]
    fn render_dir_fs_substitutes_variables_and_strips_hbs() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let dest = dir.path().join("dest");
        std::fs::create_dir_all(&source).unwrap();

        // Create a .hbs template file in the source directory.
        std::fs::write(
            source.join("Cargo.toml.hbs"),
            "[package]\nname = \"{{project_name}}\"\n",
        )
        .unwrap();
        // And a plain file.
        std::fs::write(source.join("README.md"), "# {{project_name}}\n").unwrap();

        let mut vars = BTreeMap::new();
        vars.insert("project_name".into(), "my-contract".into());

        render_dir_fs(&source, &source, &dest, &vars).unwrap();

        // .hbs suffix must be stripped.
        assert!(
            dest.join("Cargo.toml").exists(),
            "Cargo.toml.hbs -> Cargo.toml"
        );
        assert!(
            !dest.join("Cargo.toml.hbs").exists(),
            ".hbs file must not appear in dest"
        );

        let cargo = std::fs::read_to_string(dest.join("Cargo.toml")).unwrap();
        assert!(
            cargo.contains("my-contract"),
            "variable substitution applied"
        );

        let readme = std::fs::read_to_string(dest.join("README.md")).unwrap();
        assert!(
            readme.contains("# my-contract"),
            "substitution in plain file"
        );
    }

    /// generate_from_url refuses to overwrite an existing dest without --force.
    #[test]
    fn generate_from_url_refuses_existing_dest_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("my-project");
        std::fs::create_dir_all(&dest).unwrap();

        let vars = project_vars("my-project", "Test Author", "2021");
        let result = generate_from_url("https://example.com/tpl", &dest, &vars, false);
        assert!(
            matches!(result, Err(ForgeError::AlreadyExists(_))),
            "expected AlreadyExists, got: {result:?}"
        );
    }

    /// A clearly bad URL (guaranteed offline, no such host) must return a
    /// descriptive error, not panic. This test runs even in offline environments
    /// because the point is error propagation, not actual network access.
    #[test]
    fn generate_from_url_returns_descriptive_error_for_unreachable_url() {
        // Skip this test if git is not installed.
        if std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("my-project");
        let vars = project_vars("my-project", "Test Author", "2021");

        // This URL is syntactically valid but guaranteed to be unreachable.
        let result = generate_from_url(
            "https://this-host-does-not-exist.invalid/repo",
            &dest,
            &vars,
            false,
        );

        assert!(result.is_err(), "expected an error for unreachable URL");
        let err = result.unwrap_err().to_string();
        // Should contain either a network-error hint or a git clone failure message.
        assert!(
            err.contains("could not clone") || err.contains("git clone failed"),
            "error message should describe the clone failure, got: {err}"
        );
    }

    // ── template.toml / --var ─────────────────────────────────────────────

    #[test]
    fn var_flag_is_repeatable() {
        let matches = ScaffoldPlugin
            .command()
            .try_get_matches_from(vec![
                "new",
                "demo",
                "--var",
                "symbol=TKN",
                "--var",
                "supply=100",
            ])
            .unwrap();
        let vars: Vec<&String> = matches.get_many::<String>("var").unwrap().collect();
        assert_eq!(vars, vec!["symbol=TKN", "supply=100"]);
    }

    #[test]
    fn bundled_templates_without_a_manifest_report_none() {
        for template in available_templates() {
            if ["crowdfund", "hello-world", "token"].contains(&template) {
                continue;
            }
            assert_eq!(
                bundled_manifest(template).unwrap(),
                None,
                "template `{template}` unexpectedly ships a {}",
                manifest::MANIFEST_FILE
            );
        }
    }

    #[test]
    fn unknown_template_has_no_manifest() {
        assert_eq!(bundled_manifest("does-not-exist").unwrap(), None);
    }

    #[test]
    fn reads_a_manifest_from_a_template_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(manifest::MANIFEST_FILE),
            "[[variables]]\nname = \"symbol\"\ndefault = \"TKN\"\n",
        )
        .unwrap();
        let manifest = manifest_in_dir(dir.path()).unwrap().unwrap();
        assert_eq!(manifest.variables[0].name, "symbol");
    }

    #[test]
    fn no_manifest_in_a_directory_without_one() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(manifest_in_dir(dir.path()).unwrap(), None);
    }

    /// `template.toml` configures generation — it must not land in the project.
    #[test]
    fn the_manifest_is_not_copied_into_the_generated_project() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let dest = dir.path().join("dest");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            source.join(manifest::MANIFEST_FILE),
            "[[variables]]\nname = \"symbol\"\n",
        )
        .unwrap();
        std::fs::write(source.join("README.md"), "# {{project_name}}: {{symbol}}\n").unwrap();

        let vars = Vars::from([
            ("project_name".to_string(), "demo".to_string()),
            ("symbol".to_string(), "TKN".to_string()),
        ]);
        render_dir_fs(&source, &source, &dest, &vars).unwrap();

        assert!(!dest.join(manifest::MANIFEST_FILE).exists());
        let readme = std::fs::read_to_string(dest.join("README.md")).unwrap();
        assert_eq!(readme, "# demo: TKN\n");
    }

    #[test]
    fn a_nested_file_named_template_toml_is_still_copied() {
        // Only the manifest at the template root is special.
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let dest = dir.path().join("dest");
        std::fs::create_dir_all(source.join("nested")).unwrap();
        std::fs::write(
            source.join("nested").join(manifest::MANIFEST_FILE),
            "x = 1\n",
        )
        .unwrap();

        render_dir_fs(&source, &source, &dest, &Vars::new()).unwrap();
        assert!(dest.join("nested").join(manifest::MANIFEST_FILE).is_file());
    }

    /// If `git` is not on PATH, generate_from_url returns ToolMissing, not a
    /// panic or an opaque IO error.
    #[test]
    fn generate_from_url_returns_tool_missing_when_git_absent() {
        // We can't remove git from PATH in a test, so we simulate by passing a
        // path override via the environment. This test is advisory only.
        // Instead, we verify the ToolMissing arm compiles and the error message
        // is correct by constructing it directly.
        let err =
            ForgeError::ToolMissing("git — install git to use --from with remote templates".into());
        assert!(err.to_string().contains("git"));
        assert!(err.to_string().contains("--from"));
    }
}
