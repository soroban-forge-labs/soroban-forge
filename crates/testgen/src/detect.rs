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
    /// The `#[contract]` struct, e.g. `HelloContract`.
    pub contract_type: String,
    /// Whether the contract defines a `__constructor` (its registration then
    /// needs constructor arguments the generator cannot guess).
    pub has_constructor: bool,
    /// Whether dev-dependencies enable soroban-sdk's `testutils` feature.
    pub has_testutils: bool,
    /// Methods exported by the contract (found in #[contractimpl] blocks)
    pub methods: Vec<MethodInfo>,
    /// Constructor arguments (if a __constructor exists)
    pub constructor_args: Option<Vec<(String, String)>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MethodInfo {
    pub name: String,
    pub args: Vec<(String, String)>,
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

    let contract_type = find_contract_type(&source).ok_or_else(|| {
        ForgeError::Other(format!(
            "no #[contract] struct found in {} (inspected)",
            lib_path.display()
        ))
    })?;

    let (methods, constructor_args) = find_methods(&source);

    Ok(ContractInfo {
        crate_name: manifest.package.name.replace('-', "_"),
        package_name: manifest.package.name,
        contract_type,
        has_constructor: source.contains("fn __constructor"),
        has_testutils: manifest_has_testutils(&manifest.dev_dependencies),
        methods,
        constructor_args,
    })
}

/// Find the struct annotated with `#[contract]` (exactly — not
/// `#[contractimpl]` or `#[contracttype]`).
pub fn find_contract_type(source: &str) -> Option<String> {
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
                .or_else(|| line.strip_prefix("struct "))?;
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            return if name.is_empty() { None } else { Some(name) };
        }
    }
    None
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

pub fn find_methods(source: &str) -> (Vec<MethodInfo>, Option<Vec<(String, String)>>) {
    let mut methods = Vec::new();
    let mut constructor_args = None;
    let mut tokens = Vec::new();
    let mut current_word = String::new();
    for c in source.chars() {
        if c.is_alphanumeric() || c == '_' {
            current_word.push(c);
        } else {
            if !current_word.is_empty() {
                tokens.push(current_word.clone());
                current_word.clear();
            }
            if !c.is_whitespace() {
                tokens.push(c.to_string());
            }
        }
    }
    if !current_word.is_empty() {
        tokens.push(current_word);
    }
    
    let mut i = 0;
    let mut in_contract_impl = false;
    let mut brace_depth = 0;
    
    while i < tokens.len() {
        if tokens[i] == "#" && i + 3 < tokens.len() && tokens[i+1] == "[" && tokens[i+2] == "contractimpl" && tokens[i+3] == "]" {
            in_contract_impl = true;
            i += 4;
            continue;
        }
        
        if in_contract_impl && tokens[i] == "{" {
            brace_depth += 1;
        }
        
        if in_contract_impl && tokens[i] == "}" {
            brace_depth -= 1;
            if brace_depth == 0 {
                in_contract_impl = false;
            }
        }
        
        if in_contract_impl && brace_depth == 1 && tokens[i] == "fn" && i + 2 < tokens.len() {
            let name = tokens[i+1].clone();
            if tokens[i+2] == "(" {
                let mut args = Vec::new();
                let mut j = i + 3;
                let mut arg_name = String::new();
                let mut expecting_type = false;
                let mut current_type = String::new();
                let mut type_angle_depth = 0;
                
                while j < tokens.len() && tokens[j] != ")" {
                    let tok = &tokens[j];
                    if !expecting_type {
                        if tok != "," {
                            if tok == ":" {
                                expecting_type = true;
                            } else {
                                arg_name = tok.clone();
                            }
                        }
                    } else {
                        if tok == "<" {
                            type_angle_depth += 1;
                            current_type.push_str(tok);
                        } else if tok == ">" {
                            type_angle_depth -= 1;
                            current_type.push_str(tok);
                        } else if tok == "," && type_angle_depth == 0 {
                            if arg_name != "env" && arg_name != "Env" && !arg_name.is_empty() {
                                args.push((arg_name.clone(), current_type.trim().to_string()));
                            }
                            arg_name.clear();
                            current_type.clear();
                            expecting_type = false;
                        } else {
                            current_type.push_str(tok);
                        }
                    }
                    j += 1;
                }
                
                if expecting_type {
                    if arg_name != "env" && arg_name != "Env" && !arg_name.is_empty() {
                        args.push((arg_name.clone(), current_type.trim().to_string()));
                    }
                }
                
                if name == "__constructor" {
                    constructor_args = Some(args);
                } else {
                    methods.push(MethodInfo { name, args });
                }
                
                i = j;
            }
        }
        
        i += 1;
    }
    
    (methods, constructor_args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_plain_contract_struct() {
        let src = "#![no_std]\n#[contract]\npub struct HelloContract;\n";
        assert_eq!(find_contract_type(src).as_deref(), Some("HelloContract"));
    }

    #[test]
    fn skips_derives_between_attr_and_struct() {
        let src = "#[contract]\n#[derive(Clone)]\npub struct Foo {\n}";
        assert_eq!(find_contract_type(src).as_deref(), Some("Foo"));
    }

    #[test]
    fn does_not_match_contractimpl_or_contracttype() {
        let src = "#[contractimpl]\nimpl Foo {}\n#[contracttype]\npub enum DataKey { A }\n";
        assert_eq!(find_contract_type(src), None);
    }

    #[test]
    fn non_pub_struct_is_found() {
        let src = "#[contract]\nstruct Hidden;\n";
        assert_eq!(find_contract_type(src).as_deref(), Some("Hidden"));
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
        assert_eq!(info.contract_type, "DemoContract");
        assert!(info.has_constructor);
        assert!(info.has_testutils);
        assert!(info.methods.is_empty());
    }

    #[test]
    fn extracts_methods_from_contractimpl() {
        let src = r#"
#[contractimpl]
impl TokenContract {
    pub fn __constructor(env: Env, admin: Address, decimals: u32) { }
    pub fn mint(env: Env, to: Address, amount: i128) { }
    pub fn admin(env: Env) -> Address { }
}
#[contractimpl]
impl TokenInterface for TokenContract {
    fn allowance(env: Env, from: Address, spender: Address) -> i128 { }
}
        "#;
        let (methods, constructor_args) = find_methods(src);
        assert_eq!(constructor_args.unwrap(), vec![("admin".to_string(), "Address".to_string()), ("decimals".to_string(), "u32".to_string())]);
        assert_eq!(methods.len(), 3);
        assert_eq!(methods[0].name, "mint");
        assert_eq!(methods[0].args, vec![("to".to_string(), "Address".to_string()), ("amount".to_string(), "i128".to_string())]);
        
        assert_eq!(methods[1].name, "admin");
        assert_eq!(methods[1].args.len(), 0);

        assert_eq!(methods[2].name, "allowance");
        assert_eq!(methods[2].args, vec![("from".to_string(), "Address".to_string()), ("spender".to_string(), "Address".to_string())]);
    }
}
