//! Lightweight source inspection: find the `#[contract]` type and crate
//! metadata of an existing Soroban project without pulling in `syn`.

use std::path::Path;

use serde::Deserialize;
use soroban_forge_core::{ForgeError, Result};

/// What testgen learned about the target contract crate.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
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
    /// Parsed constructor arguments mapped to default/sensible values.
    pub constructor_args: String,
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

    let has_constructor = source.contains("fn __constructor");
    let constructor_args = if has_constructor {
        parse_constructor_args(&source).unwrap_or_else(|| "()".to_string())
    } else {
        "()".to_string()
    };

    Ok(ContractInfo {
        crate_name: manifest.package.name.replace('-', "_"),
        package_name: manifest.package.name,
        contract_types,
        has_constructor,
        has_testutils: manifest_has_testutils(&manifest.dev_dependencies),
        constructor_args,
    })
}

/// Parse `__constructor` arguments from the source code and generate sensible default values.
pub fn parse_constructor_args(source: &str) -> Option<String> {
    let idx = source.find("fn __constructor")?;
    let after = &source[idx + "fn __constructor".len()..];
    
    let start_paren = after.find('(')?;
    let content_after = &after[start_paren + 1..];
    
    let mut depth = 1;
    let mut end_paren = None;
    let chars: Vec<char> = content_after.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c == '(' {
            depth += 1;
        } else if c == ')' {
            depth -= 1;
            if depth == 0 {
                end_paren = Some(i);
                break;
            }
        }
    }
    
    let end_idx = end_paren?;
    let params_str: String = chars[..end_idx].iter().collect();
    
    let mut params = Vec::new();
    let mut current = String::new();
    let mut bracket_depth = 0;
    let mut paren_depth = 0;
    
    for c in params_str.chars() {
        match c {
            '<' => bracket_depth += 1,
            '>' => bracket_depth -= 1,
            '(' => paren_depth += 1,
            ')' => paren_depth -= 1,
            ',' if bracket_depth == 0 && paren_depth == 0 => {
                params.push(current.trim().to_string());
                current.clear();
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    if !current.trim().is_empty() {
        params.push(current.trim().to_string());
    }
    
    if params.is_empty() {
        return Some("()".to_string());
    }
    
    let has_env = params[0].to_lowercase().contains("env");
    let start_idx = if has_env { 1 } else { 0 };
    
    let mut generated_args = Vec::new();
    
    for param in &params[start_idx..] {
        if param.is_empty() {
            continue;
        }
        if let Some(colon_idx) = param.find(':') {
            let name = param[..colon_idx].trim();
            let ty_str = param[colon_idx + 1..].trim();
            let val = map_type_to_default(ty_str);
            generated_args.push(format!("        {val}, // {name}"));
        }
    }
    
    if generated_args.is_empty() {
        Some("()".to_string())
    } else {
        let formatted = format!("(\n{}\n    )", generated_args.join("\n"));
        Some(formatted)
    }
}

fn map_type_to_default(ty: &str) -> String {
    let ty_clean = ty.replace('&', "")
                    .replace("'a", "")
                    .replace("mut ", "")
                    .trim()
                    .to_string();
                    
    if ty_clean.contains("Address") {
        "common::new_account(&env)".to_string()
    } else if ty_clean == "i128" {
        "0_i128".to_string()
    } else if ty_clean == "u128" {
        "0_u128".to_string()
    } else if ty_clean == "i64" {
        "0_i64".to_string()
    } else if ty_clean == "u64" {
        "0_u64".to_string()
    } else if ty_clean == "i32" {
        "0_i32".to_string()
    } else if ty_clean == "u32" {
        "0_u32".to_string()
    } else if ty_clean == "i16" {
        "0_i16".to_string()
    } else if ty_clean == "u16" {
        "0_u16".to_string()
    } else if ty_clean == "i8" {
        "0_i8".to_string()
    } else if ty_clean == "u8" {
        "0_u8".to_string()
    } else if ty_clean == "bool" {
        "true".to_string()
    } else if ty_clean.ends_with("String") {
        "soroban_sdk::String::from_str(&env, \"demo\")".to_string()
    } else if ty_clean.ends_with("Symbol") {
        "soroban_sdk::Symbol::new(&env, \"demo\")".to_string()
    } else if ty_clean.starts_with("Option") {
        "None".to_string()
    } else if ty_clean.contains("Vec") {
        "soroban_sdk::vec![&env]".to_string()
    } else if ty_clean.contains("Map") {
        "soroban_sdk::map![&env]".to_string()
    } else if ty_clean.ends_with("Bytes") {
        "soroban_sdk::Bytes::new(&env)".to_string()
    } else if ty_clean.ends_with("Val") {
        "soroban_sdk::Val::from_void()".to_string()
    } else {
        "Default::default()".to_string()
    }
}

/// Find all structs annotated with `#[contract]` (exactly — not
/// `#[contractimpl]` or `#[contracttype]`).
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
        assert_eq!(info.constructor_args, "()");
    }

    #[test]
    fn parses_constructor_arguments_with_types() {
        let src = r#"
            pub fn __constructor(
                env: Env,
                owner: Address,
                decimals: u32,
                symbol: Symbol,
                metadata: Option<String>
            ) {
            }
        "#;
        let parsed = parse_constructor_args(src).unwrap();
        assert!(parsed.contains("common::new_account(&env), // owner"));
        assert!(parsed.contains("0_u32, // decimals"));
        assert!(parsed.contains("soroban_sdk::Symbol::new(&env, \"demo\"), // symbol"));
        assert!(parsed.contains("None, // metadata"));
    }
}
