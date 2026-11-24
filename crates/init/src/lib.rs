//! Initialize soroban-forge in an existing contract project.

use std::path::Path;

use clap::{Arg, ArgAction, ArgMatches, Command};
use serde::Serialize;
use soroban_forge_ci_presets::GenerateOptions;
use soroban_forge_core::{ForgeContext, ForgeError, ForgePlugin, Result};

#[derive(Serialize)]
struct ForgeFile {
    project: Project,
}

#[derive(Serialize)]
struct Project {
    name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    authors: Vec<String>,
}

fn manifest_project(dir: &Path) -> Result<Project> {
    let path = dir.join("Cargo.toml");
    let raw = std::fs::read_to_string(&path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            ForgeError::InvalidArgument(format!(
                "{} is not an existing Cargo project (Cargo.toml was not found)",
                dir.display()
            ))
        } else {
            ForgeError::io(format!("reading {}", path.display()))(err)
        }
    })?;
    let manifest: toml::Value = toml::from_str(&raw).map_err(|err| ForgeError::Config {
        path: path.clone(),
        message: err.to_string(),
    })?;
    let package = manifest.get("package").and_then(toml::Value::as_table);
    let workspace = manifest.get("workspace").and_then(toml::Value::as_table);
    let workspace_package = workspace
        .and_then(|value| value.get("package"))
        .and_then(toml::Value::as_table);
    let name = package
        .and_then(|value| value.get("name"))
        .and_then(toml::Value::as_str)
        .or_else(|| {
            workspace
                .and_then(|value| value.get("metadata"))
                .and_then(|value| value.get("name"))
                .and_then(toml::Value::as_str)
        })
        .map(ToString::to_string)
        .or_else(|| {
            dir.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .ok_or_else(|| ForgeError::Config {
            path: path.clone(),
            message: "could not determine project name".into(),
        })?;
    let package_authors = package.and_then(|value| value.get("authors"));
    let inherited_authors = package_authors
        .and_then(toml::Value::as_table)
        .and_then(|value| value.get("workspace"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(false);
    let authors = if inherited_authors {
        workspace_package.and_then(|value| value.get("authors"))
    } else {
        package_authors
    }
    .and_then(toml::Value::as_array)
    .map(|values| {
        values
            .iter()
            .filter_map(toml::Value::as_str)
            .map(ToString::to_string)
            .collect()
    })
    .unwrap_or_default();

    Ok(Project { name, authors })
}

/// Write a new `forge.toml` without changing existing project files.
pub fn initialize(dir: &Path) -> Result<String> {
    let config_path = dir.join("forge.toml");
    if config_path.exists() {
        return Err(ForgeError::AlreadyExists(config_path));
    }

    let project = manifest_project(dir)?;
    let name = project.name.clone();
    let contents = toml::to_string_pretty(&ForgeFile { project })
        .map_err(|err| ForgeError::Other(format!("serializing forge.toml: {err}")))?;
    std::fs::write(&config_path, contents)
        .map_err(ForgeError::io(format!("writing {}", config_path.display())))?;
    Ok(name)
}

/// The `init` subcommand.
pub struct InitPlugin;

impl ForgePlugin for InitPlugin {
    fn name(&self) -> &'static str {
        "init"
    }

    fn command(&self) -> Command {
        Command::new("init")
            .about("Add soroban-forge configuration to an existing contract project")
            .arg(
                Arg::new("path")
                    .long("path")
                    .help("Existing contract project directory [default: current directory]"),
            )
            .arg(
                Arg::new("tests")
                    .long("tests")
                    .alias("test")
                    .action(ArgAction::SetTrue)
                    .help("Also add the generated test harness"),
            )
            .arg(
                Arg::new("ci")
                    .long("ci")
                    .action(ArgAction::SetTrue)
                    .help("Also add the default GitHub CI workflows"),
            )
            .arg(
                Arg::new("provider")
                    .long("provider")
                    .default_value("github"),
            )
            .arg(Arg::new("deploy").long("deploy").action(ArgAction::SetTrue))
            .arg(
                Arg::new("security-scan")
                    .long("security-scan")
                    .action(ArgAction::SetTrue),
            )
            .arg(
                Arg::new("coverage")
                    .long("coverage")
                    .action(ArgAction::SetTrue),
            )
            .arg(
                Arg::new("actionlint")
                    .long("actionlint")
                    .action(ArgAction::SetTrue),
            )
            .arg(
                Arg::new("healthcheck")
                    .long("healthcheck")
                    .action(ArgAction::SetTrue),
            )
            .arg(Arg::new("matrix").long("matrix").action(ArgAction::SetTrue))
            .arg(Arg::new("stale").long("stale").action(ArgAction::SetTrue))
            .arg(
                Arg::new("dependabot")
                    .long("dependabot")
                    .action(ArgAction::SetTrue),
            )
            .arg(Arg::new("msrv").long("msrv"))
            .arg(
                Arg::new("max-size")
                    .long("max-size")
                    .value_parser(clap::value_parser!(u64)),
            )
    }

    fn run(&self, matches: &ArgMatches, ctx: &ForgeContext) -> Result<()> {
        let dir = matches
            .get_one::<String>("path")
            .map(|path| ctx.cwd.join(path))
            .unwrap_or_else(|| ctx.cwd.clone());

        let project = manifest_project(&dir)?;
        let config_path = dir.join("forge.toml");
        if config_path.exists() {
            return Err(ForgeError::AlreadyExists(config_path));
        }

        let mut test_files = Vec::new();
        if matches.get_flag("tests") {
            let (_, written, _) = soroban_forge_testgen::generate(&dir, false, false)?;
            test_files = written.into_iter().map(ToString::to_string).collect();
        }

        let mut ci_files = Vec::new();
        if matches.get_flag("ci") {
            let options = GenerateOptions {
                deploy: matches.get_flag("deploy"),
                security_scan: matches.get_flag("security-scan"),
                coverage: matches.get_flag("coverage"),
                actionlint: matches.get_flag("actionlint"),
                healthcheck: matches.get_flag("healthcheck"),
                matrix: matches.get_flag("matrix"),
                stale: matches.get_flag("stale"),
                dependabot: matches.get_flag("dependabot"),
                msrv: matches.get_one::<String>("msrv").cloned(),
                max_size: matches.get_one::<u64>("max-size").copied(),
            };
            let providers = soroban_forge_ci_presets::parse_providers(
                matches
                    .get_one::<String>("provider")
                    .expect("default provider"),
            )?;
            for provider in providers {
                match soroban_forge_ci_presets::generate(
                    &dir,
                    &provider,
                    &project.name,
                    options.deploy,
                    false,
                    &options,
                    false,
                    false,
                ) {
                    Ok(files) => ci_files.extend(files),
                    Err(error) => {
                        for file in test_files.iter().chain(ci_files.iter()) {
                            let _ = std::fs::remove_file(dir.join(file));
                        }
                        return Err(error);
                    }
                }
            }
        }

        let project_name = initialize(&dir)?;
        log::info!("initialized soroban-forge in {}", dir.display());
        if ctx.json {
            let report = serde_json::json!({
                "project_name": project_name,
                "config": "forge.toml",
                "test_files": test_files,
                "ci_files": ci_files,
            });
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
        } else if !ctx.quiet {
            println!("initialized soroban-forge in {}", dir.display());
            println!("  forge.toml");
            for path in test_files.iter().chain(ci_files.iter()) {
                println!("  {path}");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initializes_an_existing_crate_without_rewriting_its_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = "[package]\nname = \"existing-contract\"\nversion = \"0.1.0\"\nauthors = [\"A Developer\"]\n";
        std::fs::write(dir.path().join("Cargo.toml"), manifest).unwrap();

        assert_eq!(initialize(dir.path()).unwrap(), "existing-contract");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap(),
            manifest
        );
        let config = std::fs::read_to_string(dir.path().join("forge.toml")).unwrap();
        assert!(config.contains("name = \"existing-contract\""));
        assert!(config.contains("authors = [\"A Developer\"]"));
    }

    #[test]
    fn refuses_to_overwrite_an_existing_config() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"existing-contract\"\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("forge.toml"), "keep = \"me\"\n").unwrap();

        assert!(matches!(
            initialize(dir.path()),
            Err(ForgeError::AlreadyExists(_))
        ));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("forge.toml")).unwrap(),
            "keep = \"me\"\n"
        );
    }
}
