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
//! Available variables: `project_name`, `crate_name`, `author`, `sdk_version`,
//! plus any extra variables a template declares in its `template.toml` (see
//! [`manifest`]).

mod manifest;

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

/// Names of the bundled templates, sorted.
pub fn available_templates() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = TEMPLATES
        .dirs()
        .filter_map(|d| d.path().file_name().and_then(|n| n.to_str()))
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
            TemplateManifest::parse(raw, name)
        }
        None => Ok(TemplateManifest::default()),
    }
}

/// One-line description for a bundled template, or `None` for unknown names.
///
/// Reads `template.toml` first; templates without a manifest (or without a
/// `description` key) have no description here.
pub fn template_description(name: &str) -> Option<String> {
    load_manifest(name).ok().and_then(|m| m.description)
}

/// Metadata for a single bundled template.
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateInfo {
    pub name: &'static str,
    pub description: String,
}

/// Return metadata for every bundled template, sorted by name.
///
/// Designed so callers (the `templates` subcommand, future `--json` output,
/// etc.) work only with this slice — not with raw name/description pairs.
pub fn template_catalog() -> Vec<TemplateInfo> {
    available_templates()
        .into_iter()
        .map(|name| TemplateInfo {
            name,
            description: template_description(name)
                .unwrap_or_else(|| "no description available".to_string()),
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

/// Build the variable map for a project.
pub fn project_vars(project_name: &str, author: &str) -> Vars {
    let mut vars = BTreeMap::new();
    vars.insert("project_name".into(), project_name.to_string());
    vars.insert("crate_name".into(), project_name.replace('-', "_"));
    vars.insert("author".into(), author.to_string());
    vars.insert("sdk_version".into(), SOROBAN_SDK_VERSION.to_string());
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
            prompt_for(&var.prompt, &var.default)?
        } else {
            var.default.clone()
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
    write_forge_toml(dest, vars)?;
    Ok(())
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

/// Initialize a git repository in `dest`.
pub fn init_git(dest: &Path) -> Result<()> {
    let status = std::process::Command::new("git")
        .arg("init")
        .arg("--quiet")
        .arg(dest)
        .status();
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(ForgeError::Other(format!(
            "`git init` exited with status {s}"
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
                    .help("Template to use (see --list-templates)"),
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
                Arg::new("no-git")
                    .long("no-git")
                    .action(ArgAction::SetTrue)
                    .help("Skip git repository initialization"),
            )
            .arg(
                Arg::new("force")
                    .long("force")
                    .action(ArgAction::SetTrue)
                    .help("Overwrite the target directory if it exists"),
            )
            .arg(
                Arg::new("var")
                    .long("var")
                    .action(ArgAction::Append)
                    .value_name("name=value")
                    .help("Set a template-specific variable (see template.toml); repeatable"),
            )
            .arg(
                Arg::new("yes")
                    .long("yes")
                    .short('y')
                    .action(ArgAction::SetTrue)
                    .help("Never prompt; use template.toml defaults for unset variables"),
            )
    }

    fn run(&self, matches: &ArgMatches, ctx: &ForgeContext) -> Result<()> {
        if matches.get_flag("list") {
            if !ctx.quiet {
                print!("{}", format_template_list(&available_templates()));
            }
            return Ok(());
        }

        let name = matches
            .get_one::<String>("name")
            .expect("clap enforces name unless --list-templates");
        validate_project_name(name)?;

        let template = matches
            .get_one::<String>("template")
            .cloned()
            .or_else(|| {
                ctx.config
                    .as_ref()
                    .and_then(|c| c.scaffold.default_template.clone())
            })
            .unwrap_or_else(|| DEFAULT_TEMPLATE.to_string());

        let author = matches
            .get_one::<String>("author")
            .cloned()
            .unwrap_or_else(|| default_author(ctx));

        let parent = matches
            .get_one::<String>("output")
            .map(|o| ctx.cwd.join(o))
            .unwrap_or_else(|| ctx.cwd.clone());
        let dest = parent.join(name);

        let force = matches.get_flag("force");

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

        let mut vars = project_vars(name, &author);
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

        if !ctx.quiet {
            print!(
                "{}",
                format_created_report(name, &template, &dest, manifest.hints())
            );
            if matches.get_flag("pre-commit") {
                println!("  pre-commit install              # enable the git hooks");
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
        let manifest = TemplateManifest::parse(
            r#"
[[variable]]
name = "token_symbol"
prompt = "Symbol"
default = "MYT"
"#,
            "token",
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
        generate("token", &dest, &project_vars("demo", "A"), false).unwrap();
        assert!(!dest.join("template.toml").exists());
    }

    #[test]
    fn token_template_manifest_declares_expected_variables() {
        let manifest = load_manifest("token").unwrap();
        let names: Vec<&str> = manifest.variables.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["token_name", "token_symbol", "token_decimals"]);
    }

    #[test]
    fn lists_all_three_templates() {
        assert_eq!(
            available_templates(),
            vec!["crowdfund", "hello-world", "token"]
        );
    }

    #[test]
    fn template_list_report_has_heading_and_items() {
        let report = format_template_list(&["hello-world", "token"]);
        assert_eq!(report, "available templates:\n  hello-world\n  token\n");
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
        assert_eq!(names, vec!["crowdfund", "hello-world", "token"]);
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
            &project_vars("x", "A"),
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
            generate("hello-world", &dest, &project_vars("demo", "A"), false),
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
            &project_vars("demo", "Ada <ada@example.com>"),
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
    fn every_template_generates_without_leftover_hbs_files() {
        for template in available_templates() {
            let dir = tempfile::tempdir().unwrap();
            let dest = dir.path().join("proj");
            generate(template, &dest, &project_vars("proj", "A"), false).unwrap();
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
            generate(template, &dest, &project_vars("my-contract", "A"), false).unwrap();
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
        generate("hello-world", &dest, &project_vars("demo", "A"), false).unwrap();
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
        generate("hello-world", &dest, &project_vars("demo", "A"), false).unwrap();
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
        generate("hello-world", &dest, &project_vars("demo", "A"), false).unwrap();
        assert!(!dest.join(".pre-commit-config.yaml").exists());
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
}
