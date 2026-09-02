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
    /// Constructor arguments rendered as default/sensible values, e.g. `"()"`
    /// or `"(\n    common::new_account(&env), // owner\n    )"`. Fed into the
    /// smoke- and invariant-test templates as `contract_args`.
    pub constructor_args: String,
    /// Whether dev-dependencies include `proptest`.
    pub has_proptest: bool,
    /// Methods exported by the contract (found in `#[contractimpl]` blocks).
    /// Used to build the `FuzzInput` enum when generating a cargo-fuzz target.
    pub methods: Vec<MethodInfo>,
    /// Raw `(name, type)` pairs of the `__constructor` arguments, if any. Used
    /// to construct the fuzz target's constructor prototype. `None` when the
    /// contract has no `__constructor` inside a `#[contractimpl]` block.
    pub constructor_arg_types: Option<Vec<(String, String)>>,
    /// Events detected in contract methods via `env.events().publish(...)` or
    /// `#[contractevent]` attributes. Used to generate event-specific tests.
    pub events: Vec<EventInfo>,
    /// Whether the contract source contains any `#[contractevent]` attributes.
    pub has_contractevent: bool,
    /// Whether the contract source contains token address usage patterns that
    /// suggest it interacts with Stellar Asset Contract (SAC) tokens. When
    /// `true`, the generated test harness will include `create_token` / `fund`
    /// fixture helpers and register a SAC instance alongside the contract.
    pub has_token_deps: bool,
    /// Token parameter names detected in constructor or contract methods (e.g.
    /// `token`, `token_a`, `asset`). Used to generate meaningful fixture names.
    pub token_param_names: Vec<String>,
    /// The contract's initialize-style entrypoint, when it has one. Drives the
    /// generated "initializes only once" test, which asserts a second call is
    /// rejected. `None` for contracts that initialize via `__constructor`
    /// (which the host can only invoke at registration) or not at all.
    pub init_method: Option<MethodInfo>,
    /// Whether the contract reads or writes persistent storage
    /// (`env.storage().persistent()`). When `true`, a TTL / rent-extension
    /// test file is generated so rent regressions have a starting point.
    pub has_persistent_storage: bool,
    /// Whether the contract reads or writes instance storage
    /// (`env.storage().instance()`).
    pub has_instance_storage: bool,
    /// Whether the contract reads or writes temporary storage
    /// (`env.storage().temporary()`).
    pub has_temporary_storage: bool,
}

/// A contract method discovered inside a `#[contractimpl]` block.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct MethodInfo {
    /// Method name, e.g. `mint`.
    pub name: String,
    /// `(name, type)` pairs for each parameter (excluding the `env` receiver).
    pub args: Vec<(String, String)>,
}

/// A contract event detected from `env.events().publish(...)` calls or
/// `#[contractevent]` attributes.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct EventInfo {
    /// The method that publishes this event, e.g. `mint`.
    pub method: String,
    /// Topic expressions extracted from the publish call (e.g. `symbol_short!("mint")`,
    /// `from`). Used in test assertions against `env.events().all()`.
    pub topics: Vec<String>,
    /// The data expression from the publish call (e.g. `amount`, `(amount, exp)`).
    /// `None` when the event publishes only topics.
    pub data: Option<String>,
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

    let (methods, constructor_arg_types) = find_methods(&source);
    let events = find_events(&source);
    let has_contractevent = has_contractevent(&source);

    let (has_token_deps, token_param_names) = detect_token_usage(&source);
    let init_method = detect_init_method(&methods);

    Ok(ContractInfo {
        crate_name: manifest.package.name.replace('-', "_"),
        package_name: manifest.package.name,
        contract_types,
        has_constructor,
        has_testutils: manifest_has_testutils(&manifest.dev_dependencies),
        constructor_args,
        has_proptest: manifest_has_proptest(&manifest.dev_dependencies),
        methods,
        constructor_arg_types,
        events,
        has_contractevent,
        has_token_deps,
        token_param_names,
        init_method,
        has_persistent_storage: detect_persistent_storage(&source),
        has_instance_storage: detect_instance_storage(&source),
        has_temporary_storage: detect_temporary_storage(&source),
    })
}

/// Entrypoint names that conventionally perform one-time setup, most specific
/// first — the order is the tie-breaker when a contract exposes several.
pub const INIT_METHOD_NAMES: &[&str] = &["initialize", "initialise", "init", "setup"];

/// Find the contract's initialize-style entrypoint, if it has one.
///
/// Matching is on the method name only: [`MethodInfo`] carries no return type,
/// so there is no way to tell a `Result`-returning init from a panicking one —
/// which is why the generated test asserts via `try_*` rather than
/// `#[should_panic]` with a specific message.
pub fn detect_init_method(methods: &[MethodInfo]) -> Option<MethodInfo> {
    INIT_METHOD_NAMES
        .iter()
        .find_map(|candidate| methods.iter().find(|m| m.name == *candidate))
        .cloned()
}

/// True when the source touches persistent storage, e.g.
/// `env.storage().persistent().get(&key)`. Whitespace is stripped first so
/// rustfmt's method-chain wrapping (`.storage()\n    .persistent()`) still
/// matches.
pub fn detect_persistent_storage(source: &str) -> bool {
    let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
    compact.contains("storage().persistent()")
}

/// Whether the contract source reads or writes instance storage.
pub fn detect_instance_storage(source: &str) -> bool {
    let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
    compact.contains("storage().instance()")
}

/// Whether the contract source reads or writes temporary storage.
pub fn detect_temporary_storage(source: &str) -> bool {
    let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
    compact.contains("storage().temporary()")
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

pub fn map_type_to_default(ty: &str) -> String {
    let ty_clean = ty
        .replace('&', "")
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

/// Detect whether the contract source uses Stellar Asset Contract (SAC) token
/// patterns. Returns `(has_token_deps, token_param_names)`.
///
/// Detection heuristics (any one match triggers `true`):
/// - Constructor or method parameter with a name containing "token" or "asset"
///   and a type of `Address` — the classic SAC pass-by-address pattern.
/// - Source contains `TokenClient` or `StellarAssetClient` imports/usage.
/// - Source contains `register_stellar_asset_contract_v2` (already set up by
///   caller code in the same crate).
/// - Source contains `token::` namespace access (e.g. `token::Client`).
///
/// The returned `token_param_names` list collects the detected parameter names
/// (deduplicated, stable order) for use in fixture generation.
pub fn detect_token_usage(source: &str) -> (bool, Vec<String>) {
    let mut param_names: Vec<String> = Vec::new();

    // Fast checks on the raw source.
    let raw_hit = source.contains("TokenClient")
        || source.contains("StellarAssetClient")
        || source.contains("register_stellar_asset_contract_v2")
        || source.contains("token::Client")
        || source.contains("token::StellarAssetClient");

    // Walk method/constructor parameters to find Address-typed "token*" / "asset*" names.
    // Tokenise just enough to extract parameter declarations.
    let token_keywords = ["token", "asset"];

    // Simple line-by-line scan for `name: Address` patterns where `name`
    // contains a token keyword.
    for line in source.lines() {
        let line = line.trim();
        // Match patterns like `token: Address`, `token_a: Address`, `asset: Address`.
        if let Some(colon_idx) = line.find(':') {
            let name_part = line[..colon_idx].trim();
            let type_part = line[colon_idx + 1..].trim();
            // Strip trailing `,` or `)` from type.
            let type_clean: String = type_part
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '<' || *c == '>')
                .collect();
            if type_clean == "Address" || type_clean == "Address>" {
                // Only the final segment of the name (after any `mut ` prefix).
                let name = name_part.trim_start_matches("mut ").trim();
                let lower = name.to_lowercase();
                if token_keywords.iter().any(|kw| lower.contains(kw)) {
                    let owned = name.to_string();
                    if !param_names.contains(&owned) {
                        param_names.push(owned);
                    }
                }
            }
        }
    }

    let has_token_deps = raw_hit || !param_names.is_empty();
    (has_token_deps, param_names)
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

fn manifest_has_proptest(dev_dependencies: &toml::Table) -> bool {
    dev_dependencies.contains_key("proptest")
}

/// Extract the exported methods (and the `__constructor` arguments, if any)
/// from every `#[contractimpl]` block in `source`. The `env`/`Env` receiver is
/// skipped so only the real arguments remain.
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
        if tokens[i] == "#"
            && i + 3 < tokens.len()
            && tokens[i + 1] == "["
            && tokens[i + 2] == "contractimpl"
            && tokens[i + 3] == "]"
        {
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
            let name = tokens[i + 1].clone();
            if tokens[i + 2] == "(" {
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
                    } else if tok == "<" {
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
                    j += 1;
                }

                if expecting_type && arg_name != "env" && arg_name != "Env" && !arg_name.is_empty()
                {
                    args.push((arg_name.clone(), current_type.trim().to_string()));
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

/// Find `env.events().publish(...)` calls in the source and return their
/// event info (method name, topics, data).
pub fn find_events(source: &str) -> Vec<EventInfo> {
    let mut events = Vec::new();

    // Tokenize into (token_str, start_byte, end_byte)
    let mut tokens: Vec<(String, usize, usize)> = Vec::new();
    let mut current_word = String::new();
    let mut word_start = 0;

    for (i, c) in source.char_indices() {
        if c.is_alphanumeric() || c == '_' {
            if current_word.is_empty() {
                word_start = i;
            }
            current_word.push(c);
        } else {
            if !current_word.is_empty() {
                tokens.push((current_word.clone(), word_start, i));
                current_word.clear();
            }
            if !c.is_whitespace() {
                tokens.push((c.to_string(), i, i + c.len_utf8()));
            }
        }
    }
    if !current_word.is_empty() {
        tokens.push((current_word, word_start, source.len()));
    }

    let mut i = 0;
    let mut current_method: Option<String> = None;
    let mut brace_depth = 0;
    let mut in_contract_impl = false;

    while i < tokens.len() {
        if tokens[i].0 == "#"
            && i + 3 < tokens.len()
            && tokens[i + 1].0 == "["
            && tokens[i + 2].0 == "contractimpl"
            && tokens[i + 3].0 == "]"
        {
            in_contract_impl = true;
            i += 4;
            continue;
        }

        if in_contract_impl && tokens[i].0 == "{" {
            brace_depth += 1;
        }

        if in_contract_impl && tokens[i].0 == "}" {
            brace_depth -= 1;
            if brace_depth == 0 {
                in_contract_impl = false;
                current_method = None;
            }
        }

        if in_contract_impl && brace_depth == 1 && tokens[i].0 == "fn" && i + 2 < tokens.len() {
            current_method = Some(tokens[i + 1].0.clone());
        }

        // Detect `env.events().publish(...)` pattern across line breaks.
        if in_contract_impl
            && tokens[i].0 == "env"
            && i + 7 < tokens.len()
            && tokens[i + 1].0 == "."
            && tokens[i + 2].0 == "events"
            && tokens[i + 3].0 == "("
            && tokens[i + 4].0 == ")"
            && tokens[i + 5].0 == "."
            && tokens[i + 6].0 == "publish"
            && tokens[i + 7].0 == "("
        {
            if let Some(method) = &current_method {
                let open_paren_end = tokens[i + 7].2;
                let mut depth = 1;
                let mut j = i + 8;
                let mut close_paren_start = None;

                while j < tokens.len() && depth > 0 {
                    if tokens[j].0 == "(" {
                        depth += 1;
                    } else if tokens[j].0 == ")" {
                        depth -= 1;
                        if depth == 0 {
                            close_paren_start = Some(tokens[j].1);
                            break;
                        }
                    }
                    j += 1;
                }

                if let Some(end) = close_paren_start {
                    let paren_content = &source[open_paren_end..end];
                    let (topics, data) = parse_publish_args(paren_content);
                    events.push(EventInfo {
                        method: method.clone(),
                        topics,
                        data,
                    });
                }
            }
            i += 8;
            continue;
        }

        i += 1;
    }

    events
}

/// Detect `#[contractevent]` attributes in the source.
pub fn has_contractevent(source: &str) -> bool {
    source.contains("#[contractevent]")
}

/// Parse the arguments of an `env.events().publish(topics, data)` call.
/// Returns `(topics, data)` where `topics` is the list of topic expressions
/// and `data` is the data expression (if any).
fn parse_publish_args(args: &str) -> (Vec<String>, Option<String>) {
    let args = args.trim();
    if args.is_empty() {
        return (vec![], None);
    }

    let mut topics = Vec::new();
    let mut data = None;

    let mut depth = 0;
    let mut split_pos = None;

    for (idx, c) in args.char_indices() {
        match c {
            '(' | '<' | '[' => depth += 1,
            ')' | '>' | ']' => depth -= 1,
            ',' if depth == 0 => {
                split_pos = Some(idx);
                break;
            }
            _ => {}
        }
    }

    if let Some(pos) = split_pos {
        let topics_str = args[..pos].trim();
        let data_str = args[pos + 1..].trim().trim_end_matches(',').trim();
        topics = split_topics(topics_str);
        if !data_str.is_empty() {
            data = Some(data_str.to_string());
        }
    } else {
        topics = split_topics(args.trim_end_matches(',').trim());
    }

    (topics, data)
}

/// Split the topics string at the top-level commas.
/// For example, `(symbol_short!("mint"), from, to)` →
/// `["(symbol_short!(\"mint\"))", "from", "to"]`.
/// When the whole string is wrapped in a single outer paren group, the parens
/// are stripped first so that the individual topics are returned.
fn split_topics(topics: &str) -> Vec<String> {
    let topics = topics.trim();
    if topics.is_empty() {
        return vec![];
    }

    // Check if the entire string is wrapped in matching outer parens.
    let has_outer_parens = {
        let mut d = 0;
        let mut found = false;
        for (i, c) in topics.char_indices() {
            match c {
                '(' => d += 1,
                ')' => {
                    d -= 1;
                    if d == 0 && i == topics.len() - 1 {
                        found = true;
                    }
                }
                _ => {}
            }
        }
        found
    };

    let inner = if has_outer_parens {
        &topics[1..topics.len() - 1]
    } else {
        topics
    };

    let mut result = Vec::new();
    let mut current = String::new();
    let mut depth = 0;
    for c in inner.chars() {
        match c {
            '(' | '<' | '[' => {
                depth += 1;
                current.push(c);
            }
            ')' | '>' | ']' => {
                depth -= 1;
                current.push(c);
            }
            ',' if depth == 0 => {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    result.push(trimmed);
                }
                current.clear();
            }
            _ => {
                current.push(c);
            }
        }
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        result.push(trimmed);
    }
    result
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
        assert!(info.methods.is_empty());
        assert!(info.events.is_empty());
        assert!(!info.has_contractevent);
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
        assert_eq!(
            constructor_args.unwrap(),
            vec![
                ("admin".to_string(), "Address".to_string()),
                ("decimals".to_string(), "u32".to_string())
            ]
        );
        assert_eq!(methods.len(), 3);
        assert_eq!(methods[0].name, "mint");
        assert_eq!(
            methods[0].args,
            vec![
                ("to".to_string(), "Address".to_string()),
                ("amount".to_string(), "i128".to_string())
            ]
        );

        assert_eq!(methods[1].name, "admin");
        assert_eq!(methods[1].args.len(), 0);

        assert_eq!(methods[2].name, "allowance");
        assert_eq!(
            methods[2].args,
            vec![
                ("from".to_string(), "Address".to_string()),
                ("spender".to_string(), "Address".to_string())
            ]
        );
    }

    // ── event detection tests ──

    #[test]
    fn finds_events_in_contractimpl() {
        let src = r#"
#[contractimpl]
impl TokenContract {
    pub fn mint(env: Env, to: Address, amount: i128) {
        env.events().publish(
            (symbol_short!("mint"), to),
            amount,
        );
    }
    pub fn burn(env: Env, from: Address, amount: i128) {
        env.events().publish(
            (symbol_short!("burn"), from),
            amount,
        );
    }
}
"#;
        let events = find_events(src);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].method, "mint");
        assert_eq!(events[0].topics, vec!["symbol_short!(\"mint\")", "to"]);
        assert_eq!(events[0].data, Some("amount".to_string()));
        assert_eq!(events[1].method, "burn");
        assert_eq!(events[1].topics, vec!["symbol_short!(\"burn\")", "from"]);
        assert_eq!(events[1].data, Some("amount".to_string()));
    }

    #[test]
    fn finds_events_with_complex_topics() {
        let src = r#"
#[contractimpl]
impl TokenContract {
    pub fn approve(env: Env, from: Address, spender: Address, amount: i128) {
        env.events().publish(
            (symbol_short!("approve"), from, spender),
            (amount, 100u32),
        );
    }
}
"#;
        let events = find_events(src);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].method, "approve");
        assert_eq!(
            events[0].topics,
            vec!["symbol_short!(\"approve\")", "from", "spender"]
        );
        assert_eq!(events[0].data, Some("(amount, 100u32)".to_string()));
    }

    #[test]
    fn finds_events_in_multiple_impl_blocks() {
        let src = r#"
#[contractimpl]
impl TokenContract {
    pub fn mint(env: Env, to: Address, amount: i128) {
        env.events().publish((symbol_short!("mint"), to), amount);
    }
}
#[contractimpl]
impl TokenInterface for TokenContract {
    fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        env.events().publish((symbol_short!("transfer"), from, to), amount);
    }
}
"#;
        let events = find_events(src);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].method, "mint");
        assert_eq!(events[1].method, "transfer");
    }

    #[test]
    fn detects_contractevent_attribute() {
        let src = r#"
#[contractevent]
pub struct Transfer {
    pub from: Address,
    pub to: Address,
    pub amount: i128,
}
"#;
        assert!(has_contractevent(src));
    }

    #[test]
    fn no_events_when_no_publish_calls() {
        let src = r#"
#[contractimpl]
impl Foo {
    pub fn bar(env: Env) -> i32 { 42 }
}
"#;
        let events = find_events(src);
        assert!(events.is_empty());
    }

    #[test]
    fn finds_events_with_only_topics_no_data() {
        let src = r#"
#[contractimpl]
impl Contract {
    pub fn close(env: Env) {
        env.events().publish((symbol_short!("close"),), ());
    }
}
"#;
        let events = find_events(src);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].method, "close");
        assert_eq!(events[0].topics, vec!["symbol_short!(\"close\")"]);
        assert_eq!(events[0].data, Some("()".to_string()));
    }

    // ── initialize-style entrypoint detection ─────────────────────────────

    fn methods(names: &[&str]) -> Vec<MethodInfo> {
        names
            .iter()
            .map(|n| MethodInfo {
                name: (*n).to_string(),
                args: vec![],
            })
            .collect()
    }

    #[test]
    fn detects_every_init_style_name() {
        for name in INIT_METHOD_NAMES {
            let found = detect_init_method(&methods(&["balance", name, "transfer"]));
            assert_eq!(found.map(|m| m.name).as_deref(), Some(*name));
        }
    }

    #[test]
    fn prefers_initialize_over_shorter_aliases() {
        let found = detect_init_method(&methods(&["setup", "init", "initialize"]));
        assert_eq!(found.map(|m| m.name).as_deref(), Some("initialize"));
    }

    #[test]
    fn no_init_method_for_contracts_without_one() {
        assert!(detect_init_method(&methods(&["hello", "transfer"])).is_none());
    }

    #[test]
    fn init_lookalikes_are_not_matched() {
        // Only exact names count — `initialized` is a getter, not the setup call.
        assert!(detect_init_method(&methods(&["initialized", "reinit"])).is_none());
    }

    #[test]
    fn init_method_carries_its_arguments() {
        let found = detect_init_method(&[MethodInfo {
            name: "initialize".into(),
            args: vec![("admin".into(), "Address".into())],
        }])
        .unwrap();
        assert_eq!(
            found.args,
            vec![("admin".to_string(), "Address".to_string())]
        );
    }

    #[test]
    fn inspect_surfaces_the_init_method() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            r#"
#[contract]
pub struct Demo;

#[contractimpl]
impl Demo {
    pub fn initialize(env: Env, admin: Address) {}
    pub fn get(env: Env) -> u32 { 0 }
}
"#,
        )
        .unwrap();

        let info = inspect(dir.path()).unwrap();
        let init = info.init_method.expect("initialize should be detected");
        assert_eq!(init.name, "initialize");
        assert_eq!(
            init.args,
            vec![("admin".to_string(), "Address".to_string())]
        );
    }

    #[test]
    fn detects_persistent_storage_usage() {
        assert!(detect_persistent_storage(
            "env.storage().persistent().set(&key, &value);"
        ));
        // rustfmt wraps long method chains across lines.
        assert!(detect_persistent_storage(
            "env.storage()\n        .persistent()\n        .extend_ttl(&key, 100, 200);"
        ));
    }

    #[test]
    fn ignores_instance_and_temporary_only_storage() {
        assert!(!detect_persistent_storage(
            "env.storage().instance().set(&key, &value);"
        ));
        assert!(!detect_persistent_storage(
            "env.storage().temporary().extend_ttl(&key, 10, 10);"
        ));
        assert!(!detect_persistent_storage("pub fn greet(env: Env) -> u32 { 42 }"));
    }
}
