//! Lightweight source inspection: find the `#[contract]` type and crate
//! metadata of an existing Soroban project without pulling in `syn`.

use std::path::Path;

use serde::Deserialize;
use soroban_forge_core::{ForgeError, Result};

/// What testgen learned about the target contract crate.
#[derive(Debug, Clone, PartialEq)]
pub struct ContractInfo {
    /// Cargo package name, e.g. `my-project`.
    pub package_name: String,
    /// Rust crate name (snake_case), e.g. `my_project`.
    pub crate_name: String,
    /// All `#[contract]` structs found in `src/lib.rs`, e.g. `["HelloContract"]`.
    pub contract_types: Vec<String>,
    /// Whether the contract defines a `__constructor` (its registration then
    /// needs constructor arguments the generator cannot guess).
    pub has_constructor: bool,
    /// Whether dev-dependencies enable soroban-sdk's `testutils` feature.
    pub has_testutils: bool,
}

#[derive(Deserialize)]
struct Manifest {
    package: Package,
    #[serde(default, rename = "dev-dependencies")]
    dev_dependencies: toml::Table,
}

#[derive(Deserialize)]
struct Package {
    name: String,
}

/// Inspect the project at `dir` (expects `Cargo.toml` and `src/lib.rs`).
pub fn inspect(dir: &Path) -> Result<ContractInfo> {
    let manifest_path = dir.join("Cargo.toml");
    if !manifest_path.is_file() {
        return Err(ForgeError::InvalidArgument(format!(
            "{} is not a cargo project (no Cargo.toml)",
            dir.display()
        )));
    }
    let manifest_raw = std::fs::read_to_string(&manifest_path).map_err(ForgeError::io(format!(
        "reading {}",
        manifest_path.display()
    )))?;
    let manifest: Manifest = toml::from_str(&manifest_raw).map_err(|e| ForgeError::Config {
        path: manifest_path.clone(),
        message: e.to_string(),
    })?;

    let lib_path = dir.join("src/lib.rs");
    let source = std::fs::read_to_string(&lib_path)
        .map_err(ForgeError::io(format!("reading {}", lib_path.display())))?;

    let contract_types = find_contract_types(&source);
    if contract_types.is_empty() {
        return Err(ForgeError::Other(format!(
            "no #[contract] struct found in {} (inspected)",
            lib_path.display()
        )));
    }

    Ok(ContractInfo {
        crate_name: manifest.package.name.replace('-', "_"),
        package_name: manifest.package.name,
        contract_types,
        has_constructor: source.contains("fn __constructor"),
        has_testutils: manifest_has_testutils(&manifest.dev_dependencies),
    })
}

/// Find all structs annotated with `#[contract]` (exactly — not
/// `#[contractimpl]` or `#[contracttype]`). Returns them in source order.
pub fn find_contract_types(source: &str) -> Vec<String> {
    let mut results = Vec::new();
    let mut saw_contract_attr = false;
    for line in source.lines() {
        let line = line.trim();
        if line == "#[contract]" {
            saw_contract_attr = true;
            continue;
        }
        if saw_contract_attr {
            // Skip other attributes/derives between the marker and the struct.
            if line.starts_with("#[") || line.is_empty() {
                continue;
            }
            let rest = line
                .strip_prefix("pub struct ")
                .or_else(|| line.strip_prefix("struct "));
            if let Some(rest) = rest {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    results.push(name);
                }
            }
            // Reset so we can find the next #[contract] struct.
            saw_contract_attr = false;
        }
    }
    results
}

fn manifest_has_testutils(dev_dependencies: &toml::Table) -> bool {
    match dev_dependencies.get("soroban-sdk") {
        Some(toml::Value::Table(t)) => match t.get("features") {
            Some(toml::Value::Array(features)) => {
                features.iter().any(|f| f.as_str() == Some("testutils"))
            }
            _ => false,
        },
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_plain_contract_struct() {
        let src = "#![no_std]\n#[contract]\npub struct HelloContract;\n";
        assert_eq!(find_contract_types(src), vec!["HelloContract"]);
    }

    #[test]
    fn skips_derives_between_attr_and_struct() {
        let src = "#[contract]\n#[derive(Clone)]\npub struct Foo {\n}";
        assert_eq!(find_contract_types(src), vec!["Foo"]);
    }

    #[test]
    fn does_not_match_contractimpl_or_contracttype() {
        let src = "#[contractimpl]\nimpl Foo {}\n#[contracttype]\npub enum DataKey { A }\n";
        assert_eq!(find_contract_types(src), Vec::<String>::new());
    }

    #[test]
    fn non_pub_struct_is_found() {
        let src = "#[contract]\nstruct Hidden;\n";
        assert_eq!(find_contract_types(src), vec!["Hidden"]);
    }

    #[test]
    fn finds_multiple_contract_structs() {
        let src = "#[contract]\npub struct Foo;\n\n#[contract]\npub struct Bar;\n";
        assert_eq!(find_contract_types(src), vec!["Foo", "Bar"]);
    }

    #[test]
    fn finds_multiple_with_derives() {
        let src = "#[contract]\n#[derive(Clone)]\npub struct First {\n}\n\n#[contract]\npub struct Second;\n";
        assert_eq!(find_contract_types(src), vec!["First", "Second"]);
    }

    #[test]
    fn inspect_reads_manifest_and_source() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            r#"
[package]
name = "my-demo"
version = "0.1.0"
edition = "2021"

[dev-dependencies]
soroban-sdk = { version = "1", features = ["testutils"] }
"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "#[contract]\npub struct DemoContract;\nfn __constructor() {}\n",
        )
        .unwrap();

        let info = inspect(dir.path()).unwrap();
        assert_eq!(info.package_name, "my-demo");
        assert_eq!(info.crate_name, "my_demo");
        assert_eq!(info.contract_types, vec!["DemoContract"]);
        assert!(info.has_constructor);
        assert!(info.has_testutils);
    }
}
