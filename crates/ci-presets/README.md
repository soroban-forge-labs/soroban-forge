# soroban-forge-ci-presets (Module 4)

CI/CD & DevOps presets. **Owner: Person D.**

Implements the `soroban-forge ci-init --provider github` subcommand, writing
GitHub Actions workflows into the target project:

| workflow             | trigger             | what it does                                   |
|----------------------|---------------------|------------------------------------------------|
| `build-test.yml`     | push main, PR       | `cargo test` + release wasm build, and a `lint` job running `cargo fmt --all --check` + `cargo clippy --all-targets -- -D warnings` |
| `contract-size.yml`  | PR                  | fails when the wasm exceeds `MAX_WASM_KB`      |
| `testnet-deploy.yml` | manual (`--deploy`) | wraps official `stellar contract deploy`       |
| `stale.yml`          | scheduled + manual  | marks inactive issues/PRs stale and closes them after the configured grace period |
| workflow             | trigger               | what it does                                            |
|----------------------|------------------------|---------------------------------------------------------|
| `build-test.yml`     | push main, PR          | `cargo test` + release wasm build                       |
| `contract-size.yml`  | PR                     | fails when the wasm exceeds `MAX_WASM_KB`               |
| `testnet-deploy.yml` | manual (`--deploy`)    | wraps official `stellar contract deploy`                |
| `release.yml`        | tag `v*.*.*` (`--release`) | builds the wasm, verifies the build is reproducible, attaches it + a SHA256 checksum + detached signatures to a GitHub Release |
| `stale.yml`          | scheduled + manual      | marks inactive issues/PRs stale and closes them after the configured grace period |

`--dependabot` additionally writes `.github/dependabot.yml` (weekly `cargo`
and `github-actions` updates). It is a config file, not a workflow, so it
lands in `.github/` rather than `.github/workflows/`.

The global `--quiet` flag suppresses the workflow summary and deploy-secret
reminder without changing the generated workflows.

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

## Release workflow

`--release` writes a workflow triggered by pushing a tag matching `v*.*.*`
(e.g. `v0.1.0`). It builds the wasm, rebuilds it again from a clean tree to
confirm the two builds hash identically (catches non-determinism before it
reaches a release), then publishes the wasm, a `.sha256` checksum file, and
detached `.sig` files for both to a GitHub Release via
`softprops/action-gh-release`. It needs `permissions: contents: write` (the
other presets are read-only) and uses only the default `GITHUB_TOKEN` — no
additional secrets.

Consumers can verify the release assets with the published public key:

```bash
openssl dgst -sha256 -verify release-signing.pub -signature my_contract.wasm.sig my_contract.wasm
openssl dgst -sha256 -verify release-signing.pub -signature my_contract.wasm.sha256.sig my_contract.wasm.sha256
```

The private key is generated at runtime for that release and is never stored in
the repository.

## Verification

Generated YAML passes [actionlint](https://github.com/rhysd/actionlint); repo
CI runs it against freshly generated output.

## Tests

`cargo test -p soroban-forge-ci-presets`
