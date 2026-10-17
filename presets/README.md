# CI presets

Workflow templates consumed by `soroban-forge ci-init --provider <p>`.
Owned by Module 4 — see [`crates/ci-presets`](../crates/ci-presets). (updated)

Each provider is a subdirectory. `github/` is the richest one:

- `build-test.yml` — cargo test + wasm build on push/PR
- `build-test-matrix.yml` — the same job run once per Rust toolchain, stable
  plus a pinned MSRV (only written with `--matrix`)
- `build-test.yml` — cargo test + wasm build on push/PR, plus a `lint` job
  running `cargo fmt --all --check` and `cargo clippy --all-targets -- -D
  warnings`
- `contract-size.yml` — fails PRs when the built wasm exceeds a size limit
- `testnet-deploy.yml` — manual testnet deploy wrapping the official
  stellar-cli (only written with `--deploy`); references GitHub secrets, never
  stores keys
- `dependabot.yml` — weekly `cargo` and `github-actions` update PRs (only
  written with `--dependabot`); lands at `.github/dependabot.yml`, not in
  `.github/workflows/`

The other providers each carry a single build+test file mirroring
`build-test.yml`: `gitlab/.gitlab-ci.yml`, `circleci/config.yml` and
`bitbucket/bitbucket-pipelines.yml`.

`buildkite/pipeline.yml` additionally mirrors `contract-size.yml`'s size
check (a self-contained pass/fail against `MAX_WASM_BYTES`, without the
GitHub-specific PR-diff/comment step): a `build-and-test` step running
`cargo test` and the wasm release build, and a `contract-size` step —
keyed with `depends_on` so it only runs after the build succeeds — that
fails when the built wasm exceeds `MAX_WASM_BYTES`. Lands at
`.buildkite/pipeline.yml`, following Buildkite's own convention of putting
the pipeline under a `.buildkite/` directory at the project root.

Templates may use `{{project_name}}` / `{{crate_name}}` / `{{msrv}}`; GitHub's
own `${{ ... }}` expressions pass through rendering untouched.

### GitHub step overrides

To customize one generated GitHub Actions step without forking its entire
workflow, add a YAML file under
`.soroban-forge/ci-overrides/github/<workflow>/<job>/<step>.yml`. The file
contains one complete YAML list item (starting with `-`); `ci-init` substitutes
it for that preset step on normal generation, `--force` regeneration, and
`--diff` previews. Overrides remain in the project and are reapplied whenever
the workflow is regenerated.

For example, to give the Rust dependency cache in `build-test.yml` a custom
cache key, create
`.soroban-forge/ci-overrides/github/build-test/build-and-test/rust-cache.yml`:

```yaml
- uses: Swatinem/rust-cache@e18b497796c12c097a38f9edb9d0641fb99eee32  # v2
  with:
    key: rust-${{ runner.os }}-${{ hashFiles('Cargo.lock') }}
```

The workflow, job, and step IDs correspond to the `ci-init-step` markers in
the source preset files under `presets/github/`.
