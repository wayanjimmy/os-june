# Composite actions

Two composite actions live in `.github/actions/`. They're the seam between
"do CI stuff" and "do project-specific stuff" — workflows call them, they
encapsulate the Rust install and the path-diff logic. If you ever feel a
workflow getting long, the next refactor is probably "extract into a
composite".

| Action | Purpose | Used by |
|---|---|---|
| `setup-rust` | Install the toolchain from `rust-toolchain.toml`, cache cargo registry + `target/`, install mold. | `api.yml` (every job) |
| `changed-paths` | Diff against the right base; output `api`/`web` booleans. | `api.yml` (`changes` job) |

The templates ship in `assets/actions/` of this skill. Copy them into
`.github/actions/<name>/action.yml`.

## `setup-rust`

### Inputs

| Input | Default | Meaning |
|---|---|---|
| `build-profile` | `dev` | `dev`, `release`, or `test`. Determines the `target/` cache key. |
| `save-build-cache` | `"true"` | When `"false"`, restore-only — don't save back. Use for `test` builds whose artifacts would pollute cache for next `dev` build. |
| `working-directory` | `api` | Where `Cargo.lock` and `rust-toolchain.toml` live. |

### What it does

1. Installs `tomlq` (via `pip install yq`) if missing — needed to read
   `rust-toolchain.toml`.
2. Installs `rustup` if missing.
3. Reads `toolchain.channel` and `toolchain.components` from
   `rust-toolchain.toml`. Both values flow into `rustup toolchain install`
   and `rustup component add`.
4. Installs `mold` via apt for fast linking.
5. Caches the cargo registry (`~/.cargo/registry`, `~/.cargo/git`) keyed on
   `Cargo.lock`.
6. Caches `<working-directory>/target/` keyed on `<runner.os>-target-<profile>-<lockfile hash>`.
7. Caches `~/.cargo/bin/` so installed cargo tools survive.

### Why read the channel from `rust-toolchain.toml`

Two reasons:

1. **Single source of truth.** Bumping Rust = editing one file.
2. **Local/CI parity.** Whatever rustup picks locally is exactly what CI
   uses. No drift.

### Pitfalls

- **`pip install yq` only works on Linux runners.** That's fine for the
  ubuntu-latest matrix, but if you start cross-building on macOS runners,
  this step will need a `brew install yq` branch.
- **The `target/` cache key includes the profile.** A `test`-profile build
  has different compile flags than a `dev`-profile build; mixing them
  invalidates each other. The `save-build-cache: "false"` input is the
  pressure release valve for the `test` job.
- **First-run cache miss.** Expect 8–12 minutes of compile on a cold build.
  Subsequent runs are 30 seconds to a minute. If you see 8 minutes on every
  run, the cache key is being invalidated — check that `rust-toolchain.toml`
  isn't being touched.

## `changed-paths`

### Outputs

| Output | Meaning |
|---|---|
| `api` | `"true"` if `api/`-relevant paths changed. |
| `web` | `"true"` if `web/`-relevant paths changed. |

### What it does

1. Determines the right `BASE` for the diff:
   - On `pull_request.synchronize`: diff against `github.event.before` (the
     previous head of the PR branch). This lets docs-only follow-up commits
     skip API jobs that already ran on the prior head.
   - On other `pull_request` events: diff against `origin/<base_ref>`.
   - On `push`: diff against `github.event.before` (the previous main HEAD).
2. Runs `git diff --quiet BASE...HEAD -- <api-paths>` and `-- <web-paths>`.
3. Sets `api=true|false` and `web=true|false` outputs.

### The paths

```
# API-relevant paths
'api/'
'.github/actions/changed-paths/'
'.github/actions/setup-rust/'
'.github/workflows/api.yml'
'.github/workflows/build-api.yml'
'.github/workflows/promote.yml'
'Makefile'

# Portal-relevant paths (mirror for web)
'web/'
'.github/actions/changed-paths/'
'.github/workflows/web.yml'
'.github/workflows/build-web.yml'
'.github/workflows/promote.yml'
'Makefile'
```

Two judgment calls baked in:

1. **Changing the composite actions counts as touching API.** That's why
   `.github/actions/setup-rust/` is in the API list. If you tweak how Rust
   is installed, you want CI to actually run.
2. **`Makefile` counts for both.** The make targets are how CI shells out;
   any edit there could change CI behavior even when the rest of the tree
   didn't move.

If you add or restructure paths (e.g., a new top-level dir), update both
this composite's path list AND `build-api.yml`'s `on.push.paths` block.

### Why a composite, not `dorny/paths-filter`

You could use a third-party action, but the composite is small, transparent,
and uses only `git diff` + bash. Less to audit; fewer Actions Marketplace
moving parts; same behavior as running it locally.

## Composite-action mistakes

- **Forgetting `shell: bash` on a `run:` step.** Composites require it
  explicitly on every step.
- **Echoing secrets in `run:` blocks.** GitHub masks known secrets in logs,
  but only if it sees them flow through context. Constructing a token via
  string-concatenation can bypass the mask. Pass via env, not interpolation.
- **Relying on `${{ env.X }}` set in the workflow within the composite.**
  Composites have their own env scope; use `inputs.X` and pass values
  through.
- **Adding `if: failure()` at the composite level expecting it to run when
  *the workflow* fails.** It only sees its own steps. Run cleanup in the
  workflow, not the composite.
- **Inventing a composite for "deploy".** Deploy logic doesn't belong here;
  put it in a separate workflow (or repo) that consumes the GHCR tag.
