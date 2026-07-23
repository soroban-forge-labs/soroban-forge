//! # soroban-forge-templates
//!
//! `soroban-forge templates` — lists every bundled contract template with its
//! name and a one-line description.
//!
//! Template discovery and descriptions are owned by `soroban-forge-scaffold`
//! via [`scaffold::template_catalog`] — this crate is purely the presentation
//! layer, making it easy to add `--json` output later without touching the
//! discovery logic.

use clap::{ArgMatches, Command};
use soroban_forge_core::{ForgeContext, ForgePlugin, Result};
use soroban_forge_scaffold::TemplateInfo;

/// Render the template listing for terminal output.
///
/// Columns are aligned: the name column is padded to the width of the longest
/// name so descriptions line up cleanly regardless of the set of templates.
///
/// # Future JSON compatibility
///
/// This function formats the data that [`soroban_forge_scaffold::template_catalog`]
/// returns. When issue #3 (--json output) is implemented, add a parallel
/// `format_template_list_json(catalog: &[TemplateInfo]) -> String` here and
/// gate on a flag in [`TemplatesPlugin::run`] — no changes needed elsewhere.
pub fn format_template_listing(catalog: &[TemplateInfo]) -> String {
    if catalog.is_empty() {
        return "no templates available.\n".to_string();
    }

    let name_width = catalog
        .iter()
        .map(|t| t.name.len())
        .max()
        .unwrap_or(0);

    let mut out = String::from("bundled templates:\n\n");
    for entry in catalog {
        out.push_str(&format!(
            "  {:<width$}  {}\n",
            entry.name,
            entry.description,
            width = name_width,
        ));
    }
    out
}

/// The `templates` subcommand.
pub struct TemplatesPlugin;

impl ForgePlugin for TemplatesPlugin {
    fn name(&self) -> &'static str {
        "templates"
    }

    fn command(&self) -> Command {
        Command::new("templates")
            .about("List all bundled contract templates with their descriptions")
    }

    fn run(&self, _matches: &ArgMatches, ctx: &ForgeContext) -> Result<()> {
        let catalog = soroban_forge_scaffold::template_catalog();
        log::debug!("listing {} bundled templates", catalog.len());
        if !ctx.quiet {
            print!("{}", format_template_listing(&catalog));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_forge_scaffold::{available_templates, TemplateInfo};

    fn make_catalog(entries: &[(&'static str, &'static str)]) -> Vec<TemplateInfo> {
        entries
            .iter()
            .map(|(name, description)| TemplateInfo {
                name,
                description,
            })
            .collect()
    }

    // ── format_template_listing ────────────────────────────────────────────

    #[test]
    fn listing_has_header() {
        let catalog = make_catalog(&[("hello-world", "minimal greeter")]);
        let output = format_template_listing(&catalog);
        assert!(output.starts_with("bundled templates:\n"));
    }

    #[test]
    fn listing_contains_every_template_name() {
        let catalog = make_catalog(&[
            ("crowdfund", "escrow/deadline crowdfunding contract"),
            ("hello-world", "minimal greeter contract (recommended starting point)"),
            ("token", "SEP-41 fungible token"),
        ]);
        let output = format_template_listing(&catalog);
        assert!(output.contains("crowdfund"));
        assert!(output.contains("hello-world"));
        assert!(output.contains("token"));
    }

    #[test]
    fn listing_contains_every_description() {
        let catalog = make_catalog(&[
            ("crowdfund", "escrow/deadline crowdfunding contract"),
            ("hello-world", "minimal greeter contract (recommended starting point)"),
            ("token", "SEP-41 fungible token"),
        ]);
        let output = format_template_listing(&catalog);
        assert!(output.contains("escrow/deadline crowdfunding contract"));
        assert!(output.contains("minimal greeter contract"));
        assert!(output.contains("SEP-41 fungible token"));
    }

    #[test]
    fn listing_aligns_descriptions() {
        // Names of different lengths — descriptions must all start at the same column.
        let catalog = make_catalog(&[
            ("ab", "short name"),
            ("abcdefghij", "long name"),
        ]);
        let output = format_template_listing(&catalog);
        let lines: Vec<&str> = output.lines().skip(2).collect(); // skip header + blank
        // Both description columns should start at the same offset.
        let col0 = lines[0].find("short name").expect("short name not found");
        let col1 = lines[1].find("long name").expect("long name not found");
        assert_eq!(col0, col1, "descriptions are not aligned:\n{output}");
    }

    #[test]
    fn listing_empty_catalog() {
        let output = format_template_listing(&[]);
        assert_eq!(output, "no templates available.\n");
    }

    // ── integration: catalog from real embedded templates ─────────────────

    #[test]
    fn lists_all_bundled_templates() {
        let catalog = soroban_forge_scaffold::template_catalog();
        let expected = available_templates();
        let names: Vec<&str> = catalog.iter().map(|t| t.name).collect();
        assert_eq!(names, expected);
    }

    #[test]
    fn all_bundled_templates_have_non_empty_descriptions() {
        for entry in soroban_forge_scaffold::template_catalog() {
            assert!(
                !entry.description.is_empty(),
                "template `{}` has an empty description",
                entry.name
            );
            assert_ne!(
                entry.description, "no description available",
                "template `{}` is missing a real description in `template_description()`",
                entry.name
            );
        }
    }

    #[test]
    fn full_listing_output_is_well_formed() {
        let catalog = soroban_forge_scaffold::template_catalog();
        let output = format_template_listing(&catalog);

        assert!(output.starts_with("bundled templates:\n"));
        // Every template name appears in the output.
        for entry in &catalog {
            assert!(
                output.contains(entry.name),
                "name `{}` missing from listing output",
                entry.name
            );
            assert!(
                output.contains(entry.description),
                "description for `{}` missing from listing output",
                entry.name
            );
        }
    }
}
