# soroban-forge-ci-presets (Module 4)

CI/CD & DevOps presets. **Owner: Person D.**

Implements the `soroban-forge ci-init --provider github` subcommand, writing
GitHub Actions workflows into the target project:

| workflow             | trigger             | what it does                                   |
|----------------------|---------------------|------------------------------------------------|
| `build-test.yml`     | push main, PR       | `cargo test` + release wasm build              |
| `contract-size.yml`  | PR                  | fails when the wasm exceeds `MAX_WASM_KB`      |
| `testnet-deploy.yml` | manual (`--deploy`) | wraps official `stellar contract deploy`       |

## Security stance

The deploy workflow **never stores keys**. It references
`${{ secrets.STELLAR_DEPLOYER_SECRET }}` (a GitHub Actions secret the user
creates) and runs only via `workflow_dispatch` against a `testnet`
environment. A test asserts the rendered file contains the secret *reference*
and no placeholder leftovers.

## Preset format

Presets live in the top-level [`presets/`](../../presets) directory, one
subdirectory per provider, embedded at compile time. `{{project_name}}` and
`{{crate_name}}` are substituted; GitHub's own `${{ ... }}` expressions
survive rendering verbatim (guaranteed by core's renderer — unknown
placeholders pass through).

Adding a provider = new `presets/<name>/` directory + an arm in
`output_dir()` mapping it to the right path (e.g. `.gitlab-ci.yml`).

## Verification

Generated YAML passes [actionlint](https://github.com/rhysd/actionlint); repo
CI runs it against freshly generated output.

## Tests

`cargo test -p soroban-forge-ci-presets`
