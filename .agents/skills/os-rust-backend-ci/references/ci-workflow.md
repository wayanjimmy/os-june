# `api.yml` — pull-request and main checks

The PR/main CI workflow runs format / lint / test / contract-drift. It is the
*only* job that fires on pull requests, and it never publishes images. Its job
is to fail fast on a broken commit before it reaches `main`.

## Template

See [assets/workflows/api.yml](../assets/workflows/api.yml) for the full file.
Below is an annotated walkthrough — copy the asset file as-is, then read this
to understand which knobs are intentional.

```yaml
name: API

on:
  pull_request:
    types: [opened, synchronize, reopened]
  push:
    branches: [main]

env:
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: "1"
  RUSTFLAGS: "-D warnings"   # promote every warning to error in lint + test builds

concurrency:
  group: api-${{ github.ref }}
  cancel-in-progress: true   # PR-only: feature-branch force-push cancels the prior run

jobs:
  changes:
    name: Detect changes
    runs-on: ubuntu-latest
    outputs:
      api: ${{ steps.detect.outputs.api }}
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }   # need full history for `git diff`
      - id: detect
        uses: ./.github/actions/changed-paths
```

The `changes` job runs first and outputs `api: true|false`. Every downstream
job gates on `if: needs.changes.outputs.api == 'true'`. On a docs-only PR the
expensive jobs skip but the `summary` job still passes (vacuously) so required
status checks aren't held forever.

### `fmt-check`, `lint`, `test`, `contract`

Each is a separate job for parallelism. They share `setup-rust`:

```yaml
fmt-check:
  needs: changes
  if: needs.changes.outputs.api == 'true'
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - uses: ./.github/actions/setup-rust
      with:
        build-profile: dev
    - run: make api-fmt-check

lint:
  needs: changes
  if: needs.changes.outputs.api == 'true'
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - uses: ./.github/actions/setup-rust
      with:
        build-profile: dev
    - run: make api-lint            # cargo clippy --workspace --all-targets -- -D warnings
```

The Makefile targets keep the CI surface stable: bump a flag in `make
api-lint` and CI follows automatically. Don't inline `cargo` commands here —
they'd drift from local `make` runs.

### `test` — real Postgres in CI

```yaml
test:
  needs: changes
  if: needs.changes.outputs.api == 'true'
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4

    - name: Start Postgres
      run: |
        docker run -d --name postgres \
          -e POSTGRES_DB=accounts -e POSTGRES_USER=accounts -e POSTGRES_PASSWORD=accounts \
          -p 5432:5432 postgres:17 \
          -c fsync=off -c synchronous_commit=off -c full_page_writes=off \
          -c autovacuum=off -c max_connections=200 -c shared_buffers=256MB

        for i in $(seq 1 30); do
          if docker exec postgres pg_isready -U accounts -d accounts >/dev/null 2>&1; then
            echo "Postgres ready after ${i}s"; break
          fi
          sleep 1
        done
        docker exec postgres pg_isready -U accounts -d accounts

    - uses: ./.github/actions/setup-rust
      with:
        build-profile: test
        save-build-cache: "false"

    - run: make api-test
```

Two things here matter:

1. **Postgres in `docker run`, not as a `services:` block.** Both repos prefer
   the explicit `docker run` because it lets you tune the server for tests
   (turn off durability for 10× speed) and because the readiness loop avoids
   the flakiness of `services:` health checks. **Never use these flags in
   production** — `fsync=off` is fine for a CI box that gets thrown away.
2. **`save-build-cache: "false"`.** The `test` profile builds `dev` artifacts
   with `--cfg test`, which would invalidate the cache for the `lint` job on
   the next run. Restore from cache, don't save back.

### `contract`

```yaml
contract:
  needs: changes
  if: needs.changes.outputs.api == 'true'
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - uses: ./.github/actions/setup-rust
      with: { build-profile: dev }
    - run: make contract-check  # regenerates openapi.json + ts types, diffs vs committed
```

This is what stops "I forgot to regenerate the types" PRs from breaking the
frontend after merge. The `Makefile` target should:

1. Run the binary's openapi export (`cargo run -p library-api --bin export-openapi`).
2. Diff against the committed `api/openapi.json`.
3. If the frontend has generated TS types, regen those too and diff.
4. Fail if anything changed.

### `summary` — the required check

```yaml
summary:
  name: API Checks Passed
  runs-on: ubuntu-latest
  needs: [changes, fmt-check, lint, test, contract]
  if: ${{ !cancelled() }}
  steps:
    - run: |
        # log every dependency's result
        ...
        case "${{ needs.changes.outputs.api }}" in
          true)  ;;
          false) echo "no API changes; vacuous pass"; exit 0 ;;
          *)     exit 1 ;;
        esac
        if [[ "${{ needs.fmt-check.result }}" == "success" && \
              "${{ needs.lint.result }}" == "success" && \
              "${{ needs.test.result }}" == "success" && \
              "${{ needs.contract.result }}" == "success" ]]; then
          echo "All API checks passed"
        else
          exit 1
        fi
```

`summary` is the *single required status check* in branch protection. It rolls
up every other job into one green / red signal. The `case` for `api: false`
means docs-only PRs get a clean pass without queueing the heavy jobs.

`if: ${{ !cancelled() }}` ensures the summary still runs (and fails) if a
parent job was cancelled or failed — without it, cancellation propagates as a
"not run" status that some branch-protection rules treat as pending forever.

## When to extend `api.yml`

- **New code-quality checks** (e.g., `cargo deny`, `cargo audit`): add a new
  job parallel to `lint`. Wire it into `summary`'s `needs:` and the final
  `if` check.
- **Frontend lint/test**: don't bolt onto `api.yml`. Create a sibling
  `web.yml` with its own `changes.outputs.web` and `web-check`, mirroring
  the same shape. That's why `changed-paths` outputs both `api` and `web`.
- **Heavy integration suites** (load tests, end-to-end against staging): a
  separate workflow on a schedule, not on PR.

## When *not* to extend `api.yml`

- **Image build steps.** Belongs in `build-api.yml` — see
  [build-workflow.md](build-workflow.md).
- **Deploys, even to a "preview" environment.** PR previews can be tempting,
  but they invert the trust model: now a PR can publish artifacts. If you
  need preview environments later, do it from a workflow that triggers
  *after* merge to a long-lived branch (`preview`) and is gated by a label.
- **Secrets.** If a job needs a secret to run, that's a strong hint it should
  live in `build-api.yml` or `promote.yml` instead. `api.yml` should be
  runnable on a fork PR without exposing anything.

## Common `api.yml` mistakes

- **Forgetting `fetch-depth: 0`.** Without it, `git diff` against the base ref
  fails on the first PR commit. `changed-paths` needs the history.
- **Mixing required-check names.** Branch protection looks at the **job
  name**. If you rename `summary` to `gate`, you need to update branch
  protection. Stick with `summary` so the rule is stable.
- **Setting `cancel-in-progress: true` on the build/promote workflows.** PRs
  can be cancelled because the next push supersedes them. `main` builds can't
  — cancelling halfway through a `docker push` corrupts the registry layer
  state. Keep cancellation PR-only.
- **Running clippy without `-D warnings`.** Then warnings rot. The `env:
  RUSTFLAGS: "-D warnings"` block at the top promotes every warning to an
  error across `lint` *and* `test` builds, which is what you want.
- **Forgetting `RUST_BACKTRACE: "1"`.** Without it, panicking tests give you
  one-line tracebacks. Cheap to enable; expensive to debug without.
