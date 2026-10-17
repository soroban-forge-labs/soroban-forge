# soroban-forge-scaffold (Module 2)

Project scaffolding and contract templates. **Owner: Person B.**

Implements the `soroban-forge new <name> --template <t>` subcommand.

Successful creation messages and template listings honor the global
`--quiet` flag. Project generation and validation still run normally.

## Templates

Templates are plain directory trees under the repository's top-level
[`templates/`](../../templates) directory, embedded into the binary at compile
time with `include_dir`. Four ship with v0.1:

| template      | contents                                                        |
|---------------|-----------------------------------------------------------------|
| `hello-world` | Minimal greeter contract with a unit test                       |
| `nft`         | Non-fungible token with metadata, minting, transfers and burn  |
| `token`       | Fungible token implementing `soroban_sdk::token::TokenInterface` (SEP-41) |
| `crowdfund`   | Escrow/deadline crowdfunding example with success & refund paths |

Every generated project compiles with `cargo build` and passes `cargo test`
out of the box, and includes a `forge.toml` so later `soroban-forge` commands
know the project name/author.

## Template format

- File contents and file *names* may use `{{variable}}` placeholders.
  Built-in variables: `project_name`, `crate_name` (snake_case), `author`,
  `sdk_version`, `edition`.
  `sdk_version`. A template can declare its own extra variables (see below).
- A trailing `.hbs` in a file name is stripped on render. Ship manifests as
  `Cargo.toml.hbs` so cargo does not treat template dirs as packages.
- Unknown placeholders are left verbatim (see core's `render.rs`).

### Custom variables (`template.toml`)

A template may declare further variables in a `template.toml` at its root. The
file drives generation and is never copied into the generated project.

```toml
description = "a token with a configurable symbol"

[[variables]]
name = "token_symbol"
prompt = "Token symbol"
default = "TKN"

[[variables]]
name = "admin_address"
prompt = "Admin account (G...)"
required = true          # the default; set false to allow an empty value
```

Each variable is resolved from, in order:

1. `--var name=value` on the command line (repeatable).
2. An interactive prompt, when stdin and stdout are both a terminal and neither
   `--yes` nor `--json` was passed. Pressing enter accepts the default.
3. The `default` from the manifest.

A `required` variable with nothing left to fall back on is an error naming the
flag that would have supplied it — so CI and scripted runs fail fast instead of
blocking on a prompt. The five built-in variables above are derived by
soroban-forge and are rejected both in a manifest and on `--var`.
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
scaffold::generate_from_url(url, dest, &vars, force) -> Result<()>;
scaffold::generate_from_url_with(url, dest, &vars, force, &mut resolve) -> Result<()>;
scaffold::available_templates() -> Vec<&'static str>;
scaffold::project_vars(name, author, edition) -> Vars;
scaffold::bundled_manifest(template) -> Result<Option<TemplateManifest>>;
scaffold::manifest_in_dir(dir) -> Result<Option<TemplateManifest>>;
scaffold::manifest::resolve_variables(&manifest, &supplied, &mut prompter) -> Result<Vars>;
scaffold::project_vars(name, author) -> Vars;
scaffold::load_manifest(template) -> Result<TemplateManifest>;
scaffold::resolve_extra_vars(&manifest, &overrides, interactive) -> Result<Vars>;
scaffold::SOROBAN_SDK_VERSION;
```

`generate*` refuse to write into an existing directory unless `force` is set.
The confirmation the CLI shows before a forced overwrite lives in the plugin,
not in these functions — a programmatic caller that passes `force` means it.

## Tests

`cargo test -p soroban-forge-scaffold`

## Empty staking pools

The `staking` template rejects `distribute` when total stake is zero with
`Error::NoStakers` (code 5). No reward tokens move, the accumulator is unchanged,
and no `distrib` event is emitted. This also applies after the last staker withdraws.
