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

    let contract_types = find_contract_types(&source);
    if contract_types.is_empty() {
        return Err(ForgeError::Other(format!(
            "no #[contract] struct found in {} (inspected)",
            lib_path.display()
        ))
    })?;

    let (methods, constructor_args) = find_methods(&source);

    Ok(ContractInfo {
        crate_name: manifest.package.name.replace('-', "_"),
        package_name: manifest.package.name,
        contract_types,
        has_constructor,
        has_testutils: manifest_has_testutils(&manifest.dev_dependencies),
        methods,
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
