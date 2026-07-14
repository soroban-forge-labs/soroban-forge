# soroban-forge-scaffold (Module 2)

Project scaffolding and contract templates. **Owner: Person B.**

Implements the `soroban-forge new <name> --template <t>` subcommand.

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
  Available variables: `project_name`, `crate_name` (snake_case),
  `author`, `sdk_version`.
- A trailing `.hbs` in a file name is stripped on render. Ship manifests as
  `Cargo.toml.hbs` so cargo does not treat template dirs as packages.
- Unknown placeholders are left verbatim (see core's `render.rs`).

## Adding a template

1. Create `templates/<name>/` with a `Cargo.toml.hbs`, `src/lib.rs`, tests.
2. That's it — the directory name becomes the template name automatically.
3. Add a generation test in `src/lib.rs` if the template needs special checks;
   the `every_template_generates_without_leftover_hbs_files` test already
   covers the basics, and repo CI builds every generated template.

## Public surface

```rust
scaffold::generate(template, dest, &vars, force) -> Result<()>;
scaffold::available_templates() -> Vec<&'static str>;
scaffold::project_vars(name, author) -> Vars;
scaffold::SOROBAN_SDK_VERSION;
```

## Tests

`cargo test -p soroban-forge-scaffold`
