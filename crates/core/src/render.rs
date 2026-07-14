//! Minimal handlebars-style `{{variable}}` substitution.
//!
//! Shared by the scaffold and ci-presets modules. Rules:
//!
//! - `{{name}}` (optionally with inner whitespace: `{{ name }}`) is replaced
//!   when `name` is a known variable.
//! - Anything else between braces is left untouched. This is deliberate:
//!   generated GitHub Actions workflows legitimately contain
//!   `${{ secrets.FOO }}`, which must survive rendering verbatim.

use std::collections::BTreeMap;

/// Variables available to a template.
pub type Vars = BTreeMap<String, String>;

/// Render `input`, substituting known `{{variable}}` placeholders.
pub fn render_str(input: &str, vars: &Vars) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after_open = &rest[start + 2..];
        match after_open.find("}}") {
            Some(end) => {
                let inner = &after_open[..end];
                let key = inner.trim();
                if let Some(value) = vars.get(key) {
                    out.push_str(value);
                } else {
                    // Unknown placeholder (e.g. GitHub's `secrets.X`): keep verbatim.
                    out.push_str("{{");
                    out.push_str(inner);
                    out.push_str("}}");
                }
                rest = &after_open[end + 2..];
            }
            None => {
                // Unterminated braces: keep the rest verbatim.
                out.push_str("{{");
                rest = after_open;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Convenience wrapper: read a file, render its contents, write it back.
///
/// Errors are annotated with the file path for clearer diagnostics.
pub fn render_file(path: &std::path::Path, vars: &Vars) -> crate::error::Result<()> {
    let input = std::fs::read_to_string(path)
        .map_err(crate::error::ForgeError::io(format!("reading {}", path.display())))?;
    let rendered = render_str(&input, vars);
    std::fs::write(path, rendered)
        .map_err(crate::error::ForgeError::io(format!("writing {}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vars {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn replaces_known_placeholders() {
        let v = vars(&[("project_name", "demo"), ("author", "Ada")]);
        assert_eq!(
            render_str("# {{project_name}} by {{ author }}", &v),
            "# demo by Ada"
        );
    }

    #[test]
    fn keeps_unknown_placeholders_verbatim() {
        let v = vars(&[("project_name", "demo")]);
        let input = "key: ${{ secrets.STELLAR_DEPLOYER_SECRET }} for {{project_name}}";
        assert_eq!(
            render_str(input, &v),
            "key: ${{ secrets.STELLAR_DEPLOYER_SECRET }} for demo"
        );
    }

    #[test]
    fn no_placeholders_is_identity() {
        let v = Vars::new();
        let input = "fn main() { println!(\"hi\"); }";
        assert_eq!(render_str(input, &v), input);
    }

    #[test]
    fn unterminated_braces_kept() {
        let v = vars(&[("a", "1")]);
        assert_eq!(render_str("x {{a}} y {{oops", &v), "x 1 y {{oops");
    }

    #[test]
    fn adjacent_placeholders() {
        let v = vars(&[("a", "1"), ("b", "2")]);
        assert_eq!(render_str("{{a}}{{b}}", &v), "12");
    }

    #[test]
    fn empty_input_returns_empty() {
        let v = vars(&[("a", "1")]);
        assert_eq!(render_str("", &v), "");
    }

    #[test]
    fn empty_vars_leaves_input_unchanged() {
        let v = Vars::new();
        assert_eq!(render_str("{{project_name}}", &v), "{{project_name}}");
    }
}
