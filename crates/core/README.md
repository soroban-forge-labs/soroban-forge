# soroban-forge-core (Module 1)

CLI core and command framework. **Owner: Person A.**

This crate is the only shared dependency between the feature modules. It owns:

- **Argument parsing & routing** (`cli.rs`) — builds the top-level `clap`
  command from registered plugins and dispatches to them.
- **Plugin interface** (`plugin.rs`) — the `ForgePlugin` trait implemented by
  every feature module, plus `ForgeContext` (cwd, parsed config, verbosity,
  and quiet-mode state).
- **Config loading** (`config.rs`) — the optional `forge.toml` file.
- **Errors** (`error.rs`) — `ForgeError` / `Result` shared by all crates.
- **Template rendering** (`render.rs`) — minimal `{{var}}` substitution used
  by the scaffold and ci-presets modules.

## Public surface

```rust
soroban_forge_core::run(plugins: Vec<Box<dyn ForgePlugin>>) -> Result<()>;

trait ForgePlugin {
    fn name(&self) -> &'static str;          // subcommand name
    fn command(&self) -> clap::Command;      // clap definition
    fn run(&self, matches: &ArgMatches, ctx: &ForgeContext) -> Result<()>;
}
```

Adding a new module = new crate implementing `ForgePlugin` + one line in
`src/main.rs` of the root binary. Nothing in this crate needs to change.

## Design notes

- The renderer replaces only *known* variables and leaves unknown `{{…}}`
  untouched, so GitHub Actions `${{ secrets.X }}` expressions survive
  rendering (see `render.rs` tests).
- `forge.toml` is entirely optional; every field has a sensible default.
- `--quiet` is global and exposed to plugins as `ForgeContext::quiet`; plugins
  suppress successful informational reports while retaining errors and exit
  semantics.
- `--list` prints installed built-in and external subcommands; combining it with
  the global `--json` flag emits a `{"builtin":[...],"external":[...]}` object
  for scripts. Only executable files (Unix executable bit set) named
  `soroban-forge-*` on `PATH` are listed.
- Global flags given *before* an external subcommand name are forwarded to the
  `soroban-forge-<name>` binary as environment variables:
  `--verbose` → `SOROBAN_FORGE_VERBOSE=1`, `--quiet` → `SOROBAN_FORGE_QUIET=1`,
  `--json` → `SOROBAN_FORGE_JSON=1`, `--yes` → `SOROBAN_FORGE_YES=1`. Flags after
  the subcommand name are passed through on its argv as before.
- `--timeout <SECS>` (whole seconds, greater than 0; anything else is an error)
  bounds network-capable operations: HTTP calls (friendbot, Horizon) and the
  `stellar` subprocesses that talk to the network (`contract deploy`,
  `contract invoke`, `contract fetch`), which are killed when the deadline
  passes. Local-only `stellar` calls (`contract build`, `contract info
  interface`, bindings generation) and the fixed 5 s connectivity check in
  `doctor` do not use it.
- `soroban-forge completions <shell>` accepts `bash`, `zsh`, `fish` and
  `powershell`.

## Tests

`cargo test -p soroban-forge-core`
