---
name: os-rust-backend-ci
description: >-
  Use this skill when the user wants to set up, configure, debug, or extend a
  GitHub Actions pipeline for a Rust backend that runs CI checks and produces
  a container image artifact in GHCR. Scope is **testing + artifact only** —
  deployment is intentionally out of scope. Trigger on: PR checks (fmt,
  clippy, tests, OpenAPI / contract drift) for a Rust API; build-and-push on
  push-to-main; tagging images `:<short-sha>` plus mutable `:staging` /
  `:production` pointers in GHCR; a manual promote workflow that re-tags an
  existing image instead of rebuilding (build-once); rollback by re-promoting
  an older SHA; configuring `GHCR_TOKEN` / `packages: write` permissions;
  debugging `api.yml`, `build-api.yml`, `promote.yml`, or the `setup-rust` /
  `changed-paths` composite actions. Also trigger for vaguer phrasings like
  "set up CI for our notes-api", "immutable image refs with manual promote",
  or any image-tag-promotion / staging-then-production artifact pipeline on a
  Rust + GHCR stack. Do not trigger for non-Rust backends or generic Docker /
  CI questions.
---

# Open Software backend CI (Rust → GHCR)

This skill captures the **pipeline** that ships alongside `os-accounts` and
`os-platform`, with **deployment intentionally removed**. It pairs with
**`os-rust-backend`** (the code shape) — that skill defines what gets built;
this one defines how it ships as an artifact.

The pipeline in one sentence: **CI checks every PR, every push to `main`
builds an immutable `:<short-sha>` image plus a mutable `:staging` pointer in
GHCR, and a manual workflow promotes an existing image to `:production` by
re-tagging — never rebuilding.** What runs that image is the deployer's
concern; this pipeline stops at the GHCR push.

## The three workflows + two composite actions

| File | Trigger | What it does |
|---|---|---|
| `.github/workflows/api.yml` | PR + push to `main` | Lint / format / test / OpenAPI drift. Never publishes images. |
| `.github/workflows/build-api.yml` | Push to `main` touching `api/` paths, or manual dispatch | Builds + pushes `:<short-sha>` and `:staging` to GHCR. Stops there. |
| `.github/workflows/promote.yml` | Manual dispatch | Verifies an existing image, resolves it to a SHA, re-tags `:<short-sha>` → `:production` in GHCR. |
| `.github/actions/setup-rust/action.yml` | composite | Reads `rust-toolchain.toml`, installs rustup + components + mold, caches registry and `target/`. |
| `.github/actions/changed-paths/action.yml` | composite | Diffs against the right base to decide whether `api/` changed; gates all jobs. |

## Route the request first

| If the user asks for... | Open first | Non-negotiable |
|---|---|---|
| "set up CI for the API" / "add format + clippy + tests on PR" | [ci-workflow.md](references/ci-workflow.md) | `api.yml` is path-aware via `changed-paths`. PRs **never** push images. |
| "build the image on every main push" / "tag staging in GHCR" | [build-workflow.md](references/build-workflow.md) | Tag both `:<short-sha>` (immutable) and `:staging` (mutable pointer). Stop at the GHCR push. |
| "promote to production" / "manual prod tag" / "rollback" | [promote-workflow.md](references/promote-workflow.md) | Re-tag an existing image. **Never** rebuild on promote. Rollback = promote a previous SHA. |
| "what does `setup-rust` do" / "rust install in CI" | [composite-actions.md](references/composite-actions.md) | Composite reads `rust-toolchain.toml`; everything else inherits from it. |
| "what secrets / vars do I need" | [secrets-and-vars.md](references/secrets-and-vars.md) | Just GHCR via the built-in `GITHUB_TOKEN`; no third-party secrets in scope here. |
| "how do I deploy this image?" | **out of scope** — see your deployer's docs | This pipeline stops at GHCR. Add a deploy step in your own workflow that consumes the published tag. |

## Mental model: tags as pointers, SHA as identity

Every image goes to `ghcr.io/<org>/<service-name>` with these tags:

```
ghcr.io/open-software-network/os-accounts-api
    ├── :a1b2c3d                       <-  immutable artifact, the short SHA
    ├── :staging         -> a1b2c3d    <-  mutable pointer to the current staging artifact
    └── :production      -> 9z8y7x6    <-  mutable pointer to the current production artifact
```

Five rules fall out of this:

1. **Build once, promote many.** Promotion re-tags an existing image; it never
   rebuilds. Whatever bytes exercised in staging are the same bytes that get
   the `:production` marker.
2. **SHA is identity.** Anything that pins or consumes an image should pin to
   `:<short-sha>`, not to `:staging` / `:production`. Pin tools to SHAs; let
   tags float for humans.
3. **Tags are for humans and tooling.** `:staging` / `:production` are
   convenient names for "the current release" — they're not what
   production-grade consumers should pin to, they're how an operator selects
   which artifact to promote.
4. **PRs never publish.** Pull requests run checks only. No GHCR pushes. The
   image only exists once `main` has the change.
5. **Rollback uses the same workflow.** `gh workflow run promote.yml -f
   from-tag=<previous-sha>` is the rollback. No separate workflow because
   promotion already takes any source tag.

## Pipeline shape, end-to-end

```
        ┌────────────────────────────────────────────────────────────┐
        │ Pull request (any branch)                                  │
        │                                                            │
        │   api.yml                                                  │
        │     ├── changes (composite)         path-aware             │
        │     ├── fmt-check                                          │
        │     ├── lint (clippy -D warnings)                          │
        │     ├── test  (Postgres in docker, real DB)                │
        │     ├── contract  (regenerate openapi.json, diff)          │
        │     └── summary  (gate: all green or vacuous pass)         │
        │                                                            │
        │   no images. no GHCR push. no main mutation.               │
        └────────────────────────────────────────────────────────────┘

        push to main
        │
        ▼
        ┌────────────────────────────────────────────────────────────┐
        │ build-api.yml                                              │
        │   build (cargo-chef multi-stage, GHA cache scope=api)      │
        │     -> push :a1b2c3d   (immutable)                         │
        │     -> push :staging   (moves pointer)                     │
        │   STOP. The deployer's pipeline picks up from GHCR.        │
        └────────────────────────────────────────────────────────────┘

        manual dispatch (Actions → "Promote artifact to production")
        │
        ▼
        ┌────────────────────────────────────────────────────────────┐
        │ promote.yml  (inputs: artifact, from-tag=staging|<sha>)    │
        │   select                                                   │
        │   verify-images  (imagetools inspect, fail if missing)     │
        │   retag          buildx imagetools create :production      │
        │                  pointed at the verified image's digest.   │
        │   STOP.                                                    │
        └────────────────────────────────────────────────────────────┘
```

## Setup checklist (new repo)

The user is bringing this pipeline to a fresh service. Walk through this list.

1. **Confirm the backend follows the layout** in `os-rust-backend`. The CI
   assumes `api/` is the workspace root, `api/Dockerfile` exists,
   `api/openapi.json` is checked in, and there is a `make` target for each
   step (`make api-fmt-check`, `make api-lint`, `make api-test`,
   `make contract-check`).
2. **Copy the two composite actions** into `.github/actions/` — see
   [composite-actions.md](references/composite-actions.md) for the templates.
3. **Copy `api.yml`, `build-api.yml`, `promote.yml`** into `.github/workflows/`
   — see the per-workflow references for the templates. Replace
   `<service>-api` with your service name and `ghcr.io/<owner>/` if you're
   publishing under a different org.
4. **Configure GitHub** — see [secrets-and-vars.md](references/secrets-and-vars.md).
   For this scope you need almost nothing: each workflow declares its own
   `packages: write` permission so GHCR pushes work with the built-in
   `GITHUB_TOKEN`. No third-party secrets.
5. **Bootstrap the image once.** Before anything else can consume the image,
   it has to exist:
   ```bash
   gh workflow run build-api.yml
   ```
   This pushes `:<short-sha>` and `:staging` to GHCR. From here the deployer
   side of the world takes over.
6. **First production tag:**
   ```bash
   gh workflow run promote.yml -f artifact=api
   ```
   This re-tags `:staging` → `:<sha>` → `:production` in GHCR. Again, no
   deploy step happens here — that's where the deployer's tooling picks up.

## Golden rules (and why they matter)

1. **PRs run checks only.** No GHCR pushes. If a PR could push an image, an
   attacker who lands a malicious PR could escalate to the registry. The
   split keeps publishing behind the merge gate.
2. **Tags are mutable; SHAs are not.** `:staging` and `:production` move;
   `:<short-sha>` never does. Anything that wants reproducible pinning should
   pin a SHA, not a tag.
3. **The build job is separate from the promote job.** They run on different
   triggers, have different concurrency rules, and fail independently. Don't
   collapse them.
4. **`changed-paths` is the gate, not the matrix.** Path filters on `on.push`
   would skip the *workflow*; the composite skips the *job*. That difference
   matters because the "summary" job needs to exist (status check) even on
   docs-only PRs — it just passes vacuously.
5. **Concurrency groups cancel in-progress on the same ref, but not on
   `main`.** PR concurrency is `api-${{ github.ref }}` with
   `cancel-in-progress: true` so a force-push cancels the previous run. Build
   and promote workflows use `cancel-in-progress: false` to avoid killing a
   half-pushed image.
6. **Production promotion is *idempotent.*** Running it twice with the same
   inputs is harmless. Re-running after a partial failure is the recovery
   path. Don't add "do this only once" guards.
7. **The build cache is scoped per service.** `cache-to: type=gha,scope=api`
   keeps services from evicting each other. Use the same scope on
   `cache-from`.
8. **Pre-push validation matters.** Before pushing a change to `.github/`
   or `api/Dockerfile`, run `actionlint`, `bash -n` any inline shell, and a
   local `docker build` to verify the Dockerfile. A broken workflow is a
   broken main branch.
9. **Deployment lives in the deployer's repo.** Questions about how the
   image reaches a running environment are not CI questions; do not invent a
   deploy step. The `:production` tag exists exactly so the deployer has a
   stable pointer to consume without coupling them to our build pipeline.

## Files this skill ships

```
assets/
├── workflows/
│   ├── api.yml                   # PR + push checks
│   ├── build-api.yml             # main → GHCR (push only, no deploy)
│   └── promote.yml               # manual dispatch → re-tag :production
└── actions/
    ├── setup-rust/action.yml
    └── changed-paths/action.yml
```

Copy them as-is for a starting point; the per-workflow references explain
which knobs you'll usually change per service.

## Common pipeline mistakes

- **Adding image-publish steps to `api.yml`.** PR CI must not push. Keep the
  build workflow separate.
- **Using `:latest` or `:main-latest` instead of `:<sha>`.** "Latest" is the
  classic anti-pattern — environments drift, rollback is impossible, and
  Docker layer cache eviction becomes silent corruption.
- **Adding a deploy step to `build-api.yml` or `promote.yml`.** Out of scope
  for this skill. If you need one, that's a separate workflow (or a separate
  repo) that consumes the GHCR tag — keep this pipeline a pure artifact
  pipeline.
- **Rebuilding on promote.** Every rebuild is a chance to ship different
  bytes than what staging exercised. The whole point of GHCR-as-staging is
  that promote is just `imagetools create --tag :production <digest>`.
- **One concurrency group for all branches.** Use `${{ github.ref }}` so
  feature-branch pushes don't cancel `main` runs and vice versa.
- **Caching `target/` for the docker build job.** The Dockerfile's
  cargo-chef layer cache is the right place; host-side `target/` cache
  fights with it.
- **Hardcoding the Rust version in `setup-rust/action.yml`.** It reads
  `rust-toolchain.toml` — when you bump Rust, you bump that one file and CI
  follows. Don't add a redundant input.
- **Skipping `actionlint` locally.** GitHub Actions YAML is full of subtle
  traps (heredocs in `run`, secret context in `if`, matrix strategy types).
  `actionlint` catches them before push.
