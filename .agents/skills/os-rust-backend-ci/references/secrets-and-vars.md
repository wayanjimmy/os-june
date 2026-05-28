# Secrets and variables

In this scope (testing + artifact only), the pipeline needs **no
third-party secrets at all**. Everything authenticates with the built-in
`GITHUB_TOKEN` that GitHub issues per workflow run.

You can fork the repo, copy the workflows, and CI just works. No tokens to
mint, no rotation, no environment scoping.

## What you configure on the GitHub side

| Setting | Where | Value |
|---|---|---|
| Workflow permissions | Each workflow's top-level `permissions:` block | `packages: write` on `build-api.yml` and `promote.yml`; nothing extra on `api.yml`. |
| Repo-level Actions setting | Settings → Actions → General → Workflow permissions | Either set to "Read and write" (broad) OR leave as "Read-only" and rely on the per-workflow `permissions:` block (recommended — more secure). |

That's the whole list. There are **no secrets to add** in the
Settings → Secrets and variables → Actions panel for this pipeline as
shipped. (If you add a deploy step later, that's where its secrets go —
but it lives in a separate workflow.)

## GHCR auth

The build and promote workflows authenticate to GHCR with the built-in
`GITHUB_TOKEN`:

```yaml
- uses: docker/login-action@v3
  with:
    registry: ghcr.io
    username: ${{ github.actor }}
    password: ${{ secrets.GITHUB_TOKEN }}
```

For this to work the workflow needs `packages: write`, which the templates
declare per-workflow:

```yaml
# build-api.yml + promote.yml top-level
permissions:
  contents: read
  packages: write   # GHCR push / retag
```

`api.yml` doesn't need any elevated permissions; the implicit
`contents: read` is enough.

The first push to `ghcr.io/<owner>/<service>-api` creates the package.
GHCR makes it private by default; change to public on the package's
settings page if other repos or the public need to pull it. (For internal
use you'll typically keep it private and have the deployer authenticate as
a service account with read scope.)

## When the build workflow needs build-time secrets

If your Dockerfile needs **build-time secrets** (e.g., a private cargo
registry token), pass them with `docker/build-push-action`'s `secrets:`
input — not build args:

```yaml
- uses: docker/build-push-action@v6
  with:
    context: ./api
    file: ./api/Dockerfile
    push: true
    secrets: |
      "cargo_token=${{ secrets.CARGO_TOKEN }}"
```

Inside the Dockerfile, mount them at build time:

```dockerfile
RUN --mount=type=secret,id=cargo_token,target=/run/secrets/cargo_token \
    CARGO_REGISTRIES_OS_TOKEN=$(cat /run/secrets/cargo_token) cargo build --release
```

Build args land in the image's metadata (`docker history`) and leak. Build
secrets don't.

## Forks and PRs

GitHub deliberately doesn't expose secrets to PRs from forks. That's why
`api.yml` doesn't touch any secret — it runs untouched on fork PRs. The
build and promote workflows can't be triggered from fork PRs anyway
(`workflow_dispatch` isn't available, `push` to your `main` requires write
permissions), so the secret-scoping is consistent end-to-end.

If you ever add a job to `api.yml` that needs a secret, **don't.** Move it
to a workflow that triggers `on: push` to your own branches only. That's
the only way to keep PR CI safe to run on forks.

## Common secret/var mistakes

- **Adding cloud-provider or deploy tokens to *this* pipeline.** Out of
  scope. Put them in whatever workflow consumes the GHCR tag.
- **Logging `${{ secrets.X }}` to debug.** GitHub masks the literal value,
  but only if it appears exactly; constructed values (`${secret}-suffix`)
  leak. Just don't log secrets.
- **Storing the GHCR image visibility (public/private) in a workflow.**
  Visibility lives in GHCR package settings. The workflow only pushes.
- **Granting `packages: write` at the repo default level** when only the
  build + promote workflows need it. Per-workflow scoping is more secure.
