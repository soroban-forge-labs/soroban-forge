//! # soroban-forge-ci-presets
//!
//! `soroban-forge ci-init --provider github` — writes CI/CD workflows for a
//! Soroban contract project:
//!
//! - `build-test.yml`: cargo test + wasm build on push/PR
//! - `contract-size.yml`: fails PRs when the built wasm exceeds a limit
//! - `testnet-deploy.yml` (with `--deploy`): manual testnet deploy wrapping
//!   the official stellar-cli; references GitHub secrets, never stores keys.
//!
//! Presets live in the repository's top-level `presets/<provider>/` directory
//! and are embedded at compile time. `{{project_name}}` / `{{crate_name}}`
//! are substituted; GitHub's own `${{ ... }}` expressions pass through
//! untouched (see core's renderer).

use std::path::Path;

use clap::{Arg, ArgAction, ArgMatches, Command};
use include_dir::{include_dir, Dir};
use serde::Deserialize;
use soroban_forge_core::render::{render_str, Vars};
use soroban_forge_core::{ForgeContext, ForgeError, ForgePlugin, Result};

static PRESETS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../presets");

/// Workflows always written.
const BASE_WORKFLOWS: &[&str] = &["build-test.yml", "contract-size.yml"]; // base workflows
/// Workflow written only with `--deploy`.
const DEPLOY_WORKFLOW: &str = "testnet-deploy.yml";

/// Providers with a preset directory, sorted.
pub fn available_providers() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = PRESETS
        .dirs()
        .filter_map(|d| d.path().file_name().and_then(|n| n.to_str()))
        .collect();
    names.sort_unstable();
    names
}

/// Where a provider's workflows are written, relative to the project root.
pub fn output_dir(provider: &str) -> &'static str {
    match provider {
        "github" => ".github/workflows",
        _ => unreachable!("validated against available_providers()"),
    }
}

#[derive(Deserialize)]
struct Manifest {
    package: Package,
}

#[derive(Deserialize)]
struct Package {
    name: String,
}

/// Resolve the project name: `forge.toml` first, then `Cargo.toml`, then the
/// directory name.
fn project_name(dir: &Path, ctx: &ForgeContext) -> String {
    if let Some(name) = ctx.config.as_ref().and_then(|c| c.project.name.clone()) {
        return name;
    }
    let manifest_path = dir.join("Cargo.toml");
    if let Ok(raw) = std::fs::read_to_string(&manifest_path) {
        if let Ok(manifest) = toml::from_str::<Manifest>(&raw) {
            return manifest.package.name;
        }
    }
    dir.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "contract".to_string())
}

/// Write the workflows for `provider` into `dir`. Public API behind
/// `ci-init`. Returns the paths written, relative to `dir`.
pub fn generate(
    dir: &Path,
    provider: &str,
    project_name: &str,
    deploy: bool,
    force: bool,
) -> Result<Vec<String>> {
    let provider_dir = PRESETS.get_dir(provider).ok_or_else(|| {
        ForgeError::InvalidArgument(format!(
            "unknown provider `{provider}` (available: {})",
            available_providers().join(", ")
        ))
    })?;

    let mut vars = Vars::new();
    vars.insert("project_name".into(), project_name.to_string());
    vars.insert("crate_name".into(), project_name.replace('-', "_"));

    let out_rel = output_dir(provider);
    let out_dir = dir.join(out_rel);
    std::fs::create_dir_all(&out_dir)
        .map_err(ForgeError::io(format!("creating {}", out_dir.display())))?;

    let mut selected: Vec<&str> = BASE_WORKFLOWS.to_vec();
    if deploy {
        selected.push(DEPLOY_WORKFLOW);
    }

    let mut written = Vec::new();
    for name in selected {
        let file = provider_dir
            .get_file(format!("{provider}/{name}"))
            .ok_or_else(|| {
                ForgeError::Template(format!("missing preset file {provider}/{name}"))
            })?;
        let contents = file
            .contents_utf8()
            .ok_or_else(|| ForgeError::Template(format!("preset {name} is not UTF-8")))?;

        let out_path = out_dir.join(name);
        if out_path.exists() && !force {
            return Err(ForgeError::AlreadyExists(out_path));
        }
        std::fs::write(&out_path, render_str(contents, &vars))
            .map_err(ForgeError::io(format!("writing {}", out_path.display())))?;
        written.push(format!("{out_rel}/{name}"));
    }
    Ok(written)
}

/// Render the report for generated CI workflows.
pub fn format_report(
    provider: &str,
    name: &str,
    written: &[impl AsRef<str>],
    deploy: bool,
) -> String {
    let mut out = format!("wrote {provider} workflows for `{name}`:\n");
    for path in written {
        out.push_str(&format!("  {}\n", path.as_ref()));
    }
    if deploy {
        out.push_str("\nthe deploy workflow needs a GitHub secret named STELLAR_DEPLOYER_SECRET\n");
        out.push_str("(a funded testnet account's secret key). Add it under:\n");
        out.push_str("  repo → Settings → Secrets and variables → Actions\n");
    }
    out
}

/// The `ci-init` subcommand.
pub struct CiPresetsPlugin;

impl ForgePlugin for CiPresetsPlugin {
    fn name(&self) -> &'static str {
        "ci-init"
    }

    fn command(&self) -> Command {
        Command::new("ci-init")
            .about("Write CI/CD workflows (build+test, contract-size, optional testnet deploy)")
            .arg(
                Arg::new("provider")
                    .long("provider")
                    .default_value("github")
                    .help("CI provider (only `github` in v0.1)"),
            )
            .arg(
                Arg::new("deploy")
                    .long("deploy")
                    .action(ArgAction::SetTrue)
                    .help("Also write the manual testnet-deploy workflow"),
            )
            .arg(
                Arg::new("path")
                    .long("path")
                    .help("Project directory [default: current directory]"),
            )
            .arg(
                Arg::new("force")
                    .long("force")
                    .action(ArgAction::SetTrue)
                    .help("Overwrite existing workflow files"),
            )
    }

    fn run(&self, matches: &ArgMatches, ctx: &ForgeContext) -> Result<()> {
        let provider = matches.get_one::<String>("provider").expect("has default");
        let dir = matches
            .get_one::<String>("path")
            .map(|p| ctx.cwd.join(p))
            .unwrap_or_else(|| ctx.cwd.clone());
        let name = project_name(&dir, ctx);
        let deploy = matches.get_flag("deploy");

        let written = generate(&dir, provider, &name, deploy, matches.get_flag("force"))?;

        if ctx.json {
            let report = serde_json::json!({
                "provider": provider,
                "project_name": name,
                "written_files": written,
                "deploy_enabled": deploy
            });
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
        } else if !ctx.quiet {
            print!("{}", format_report(provider, &name, &written, deploy));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_is_the_only_provider() {
        assert_eq!(available_providers(), vec!["github"]);
    }

    #[test]
    fn report_lists_provider_project_and_files() {
        let report = format_report("github", "demo", &["a.yml", "b.yml"], false);
        assert_eq!(
            report,
            "wrote github workflows for `demo`:\n  a.yml\n  b.yml\n"
        );
    }

    #[test]
    fn deploy_report_explains_required_secret() {
        let report = format_report("github", "demo", &["deploy.yml"], true);
        assert!(report.contains("STELLAR_DEPLOYER_SECRET"));
        assert!(report.contains("Settings → Secrets and variables → Actions"));
    }

    #[test]
    fn base_report_omits_deploy_guidance() {
        let report = format_report("github", "demo", &["build.yml"], false);
        assert!(!report.contains("STELLAR_DEPLOYER_SECRET"));
    }

    #[test]
    fn unknown_provider_error_lists_available() {
        let dir = tempfile::tempdir().unwrap();
        let err = generate(dir.path(), "gitlab", "demo", false, false).unwrap_err();
        assert!(err.to_string().contains("github"));
    }

    #[test]
    fn writes_base_workflows() {
        let dir = tempfile::tempdir().unwrap();
        let written = generate(dir.path(), "github", "demo", false, false).unwrap();
        assert_eq!(
            written,
            vec![
                ".github/workflows/build-test.yml",
                ".github/workflows/contract-size.yml"
            ]
        );
        let build =
            std::fs::read_to_string(dir.path().join(".github/workflows/build-test.yml")).unwrap();
        assert!(build.contains("demo: build & test"));
        assert!(build.contains("wasm32v1-none"));
        assert!(!dir
            .path()
            .join(".github/workflows/testnet-deploy.yml")
            .exists());
    }

    #[test]
    fn deploy_workflow_references_secrets_only() {
        let dir = tempfile::tempdir().unwrap();
        generate(dir.path(), "github", "my-project", true, false).unwrap();
        let deploy =
            std::fs::read_to_string(dir.path().join(".github/workflows/testnet-deploy.yml"))
                .unwrap();
        // GitHub expression survives our renderer verbatim.
        assert!(deploy.contains("${{ secrets.STELLAR_DEPLOYER_SECRET }}"));
        // Crate name substituted into the wasm path.
        assert!(deploy.contains("my_project.wasm"));
        // No leftover soroban-forge placeholders.
        assert!(!deploy.contains("{{project_name}}"));
        assert!(!deploy.contains("{{crate_name}}"));
    }

    #[test]
    fn refuses_overwrite_without_force() {
        let dir = tempfile::tempdir().unwrap();
        generate(dir.path(), "github", "demo", false, false).unwrap();
        assert!(matches!(
            generate(dir.path(), "github", "demo", false, false),
            Err(ForgeError::AlreadyExists(_))
        ));
        generate(dir.path(), "github", "demo", false, true).unwrap();
    }
}
