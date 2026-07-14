# Architecture

```
                 ┌──────────────────────────────┐
                 │      soroban-forge (bin)     │   src/main.rs — wires plugins
                 └──────────────┬───────────────┘
                                │ Vec<Box<dyn ForgePlugin>>
                 ┌──────────────▼───────────────┐
                 │     soroban-forge-core       │   Module 1
                 │ clap routing · forge.toml    │
                 │ ForgePlugin trait · errors   │
                 │ {{var}} renderer             │
                 └──┬────────┬────────┬─────┬───┘
          implements│        │        │     │
   ┌────────────────▼──┐ ┌───▼─────┐ ┌▼────────────┐ ┌─▼─────────┐
   │ scaffold («new»)  │ │ testgen │ │ ci-presets  │ │ doctor    │
   │ Module 2          │ │ Module 3│ │ Module 4    │ │ Module 5  │
   └───────┬───────────┘ └─────────┘ └──────┬──────┘ └───────────┘
           │ embeds (include_dir)           │ embeds
   ┌───────▼───────────┐            ┌───────▼──────┐
   │ templates/        │            │ presets/     │
   │ hello-world       │            │ github/      │
   │ token · crowdfund │            └──────────────┘
   └───────────────────┘
```

## Key decisions

- **One trait, no cross-dependencies.** Feature crates depend only on core
  and meet at `ForgePlugin`. The binary is the sole place that knows every
  module. (Exception: testgen *dev*-depends on scaffold to test against real
  generated projects.)
- **Templates/presets are data, embedded at compile time** with `include_dir`.
  Adding a template or provider means adding files, barely any code.
- **Renderer leaves unknown `{{…}}` untouched**, so GitHub's `${{ secrets.X }}`
  expressions survive preset rendering. Template manifests ship as
  `Cargo.toml.hbs` (suffix stripped on render) so cargo ignores them.
- **Wrap, don't reimplement.** Deploys go through the official stellar-cli;
  forge never talks to the network itself.
- **Every generated artifact is verified in CI**: templates compile and pass
  tests, `test-init` output passes, `ci-init` output passes actionlint.
