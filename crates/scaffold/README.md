# soroban-forge-scaffold (Module 2)

Project scaffolding and contract templates. **Owner: Person B.**

Implements the `soroban-forge new <name> --template <t>` subcommand.

Successful creation messages and template listings honor the global
`--quiet` flag. Project generation and validation still run normally.

## Templates

Templates are plain directory trees under the repository's top-level
[`templates/`](../../templates) directory, embedded into the binary at compile
time with `include_dir`. Three ship with v0.1:

| template      | contents                                                        |
|---------------|-----------------------------------------------------------------|
| `hello-world` | Minimal greeter contract with a unit test                       |
| `token`       | Fungible token implementing `soroban_sdk::token::TokenInterface` (SEP-41) |
| `crowdfund`   | Escrow/deadline crowdfunding example with success & refund paths |

Every generated project compiles with `cargo build` and passes `cargo test`
out of the box, and includes a `forge.toml` so later `soroban-forge` commands
know the project name/author.

## Template format

- File contents and file *names* may use `{{variable}}` placeholders.
  Built-in variables: `project_name`, `crate_name` (snake_case), `author`,
  `sdk_version`. A template can declare its own extra variables (see below).
- A trailing `.hbs` in a file name is stripped on render. Ship manifests as
  `Cargo.toml.hbs` so cargo does not treat template dirs as packages.
- Unknown placeholders are left verbatim (see core's `render.rs`).

## `template.toml`

Each template directory carries an optional `template.toml` (parsed by
[`manifest.rs`](src/manifest.rs); never copied into the generated project):

```toml
description = "SEP-41 fungible token"

[[variable]]
name = "token_symbol"
prompt = "Token symbol"
default = "MYT"

[post_generate]
hints = ["the constructor takes admin, decimals, name, symbol"]
```

- `description` — shown by `soroban-forge templates` and `new --list-templates`.
- `[[variable]]` — extra `{{name}}` placeholders the template uses, beyond the
  built-ins. Each resolves in this order: `--var name=value` (repeatable) →
  an interactive stdin prompt (only when stdout/stdin are a terminal and
  neither `--quiet` nor `--yes` was passed) → the declared `default`. This
  means CI and scripted use never blocks waiting for input.
- `[post_generate] hints` — printed after "next steps" in the creation report.

A template without a `template.toml` still works — it just has no
description, no extra variables, and no hints (`TemplateManifest::default()`).

## Adding a template

1. Create `templates/<name>/` with a `Cargo.toml.hbs`, `src/lib.rs`, tests,
   and a `template.toml` with at least a `description`.
2. That's it — the directory name becomes the template name automatically.
3. Add a generation test in `src/lib.rs` if the template needs special checks;
   the `every_template_generates_without_leftover_hbs_files` test already
   covers the basics, and repo CI builds every generated template.

## Public surface

```rust
scaffold::generate(template, dest, &vars, force) -> Result<()>;
scaffold::available_templates() -> Vec<&'static str>;
scaffold::project_vars(name, author) -> Vars;
scaffold::load_manifest(template) -> Result<TemplateManifest>;
scaffold::resolve_extra_vars(&manifest, &overrides, interactive) -> Result<Vars>;
scaffold::SOROBAN_SDK_VERSION;
```

## Tests

`cargo test -p soroban-forge-scaffold`
