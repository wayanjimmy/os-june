# `build-api.yml` — main → GHCR

The build workflow runs only on push to `main` and only when API-relevant
paths change. It produces two GHCR tags and **stops there** — deployment is
out of scope for this skill (see [SKILL.md](../SKILL.md)).

## What it produces

```
ghcr.io/<owner>/<service>-api:<short-sha>     # immutable, the SHA-tagged artifact
ghcr.io/<owner>/<service>-api:staging         # mutable pointer, moved by this workflow
```

That's it. A deployer-side workflow (in whichever repo / system owns
running the service) is expected to pick up the `:staging` tag or the SHA
ref and roll it out.

## Template

See [assets/workflows/build-api.yml](../assets/workflows/build-api.yml) for the
full file. Walkthrough below — the parts you'll edit per service are
**bolded**.

```yaml
name: Build & push API image

on:
  push:
    branches: [main]
    paths:
      - 'api/**'                            # change with your workspace dir
      - '.github/workflows/build-api.yml'
  workflow_dispatch:

concurrency:
  group: build-api-${{ github.ref }}
  cancel-in-progress: false                 # NEVER cancel a half-pushed image

permissions:
  contents: read
  packages: write                           # GHCR push

env:
  REGISTRY: ghcr.io
  IMAGE: ghcr.io/${{ github.repository_owner }}/<service>-api
```

The path filter and the `IMAGE` env are the two things to edit. The path
filter governs which file changes trigger a rebuild — make sure it covers
your Dockerfile, your workspace, and anything else that meaningfully
changes the image bytes.

### `build` job

```yaml
jobs:
  build:
    name: Build & push
    runs-on: ubuntu-latest
    outputs:
      digest: ${{ steps.build.outputs.digest }}
      sha_short: ${{ steps.meta.outputs.sha_short }}
    steps:
      - uses: actions/checkout@v4

      - name: Compute metadata
        id: meta
        run: |
          sha_short=$(git rev-parse --short=7 HEAD)
          echo "sha_short=${sha_short}" >> "$GITHUB_OUTPUT"
          # head_commit.timestamp is null on workflow_dispatch — fall back to now.
          created="${{ github.event.head_commit.timestamp }}"
          if [[ -z "$created" ]]; then
            created=$(date -u +%Y-%m-%dT%H:%M:%SZ)
          fi
          echo "created=${created}" >> "$GITHUB_OUTPUT"

      - uses: docker/setup-buildx-action@v3

      - uses: docker/login-action@v3
        with:
          registry: ${{ env.REGISTRY }}
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}

      - name: Build and push
        id: build
        uses: docker/build-push-action@v6
        with:
          context: ./api
          file: ./api/Dockerfile
          push: true
          tags: |
            ${{ env.IMAGE }}:${{ steps.meta.outputs.sha_short }}
            ${{ env.IMAGE }}:staging
          labels: |
            org.opencontainers.image.source=https://github.com/${{ github.repository }}
            org.opencontainers.image.revision=${{ github.sha }}
            org.opencontainers.image.created=${{ steps.meta.outputs.created }}
          cache-from: type=gha,scope=api
          cache-to: type=gha,scope=api,mode=max
          provenance: true
          sbom: true

      - name: Summary
        run: |
          {
            echo "### API image built"
            echo "- **Tag:** \`${{ env.IMAGE }}:${{ steps.meta.outputs.sha_short }}\`"
            echo "- **Digest:** \`${{ steps.build.outputs.digest }}\`"
            echo "- **Commit:** ${{ github.sha }}"
          } >> "$GITHUB_STEP_SUMMARY"
```

Key choices and why:

1. **`sha_short = git rev-parse --short=7 HEAD`.** Always 7 chars — makes for
   clean log lines and short Docker tags.
2. **`tags:` lists both the SHA and `:staging`.** The push pushes once; both
   tags reference the same manifest digest. There's no second `docker tag`
   step.
3. **`org.opencontainers.image.revision = github.sha`.** This is the
   load-bearing label — `promote.yml` reads it to resolve `:staging` back to
   the SHA when promoting. Don't strip it.
4. **`org.opencontainers.image.created`** is computed in the meta step with
   a fallback to `date -u`. `github.event.head_commit.timestamp` is `null`
   on `workflow_dispatch`, so referencing it directly in `labels:` would
   stamp manually-triggered builds with an empty timestamp. The fallback
   keeps the label populated on every trigger path.
5. **`cache-from`/`cache-to: type=gha,scope=api`.** GitHub Actions cache,
   scoped per service so `build-api.yml` and (if you have one) `build-web.yml`
   don't fight. `mode=max` keeps intermediate cargo-chef layers.
6. **`provenance: true, sbom: true`.** Free supply-chain hardening. Buildx
   attaches an SBOM and provenance attestation; consumers can verify them.

### No deploy job

The deployer's pipeline — whatever it is — consumes the published GHCR tag.
This workflow's contract with the rest of the world is just: **after this
runs, `ghcr.io/.../<service>-api:<sha>` and `:staging` are pushed.**

## How `:staging` gets moved

Two pushes happen in one `docker buildx build --push` step:

1. **`ghcr.io/<owner>/<service>-api:<short-sha>`** — new manifest, immutable.
2. **`ghcr.io/<owner>/<service>-api:staging`** — points at the same manifest
   digest, mutating the previous pointer.

GHCR is a content-addressable store: the manifest digest is what's permanent;
the tag is a label that can be reassigned. Re-running `build-api.yml`
produces a new SHA and re-points `:staging`. The previous SHA still exists —
that's what makes rollback work.

## Common `build-api.yml` mistakes

- **`cancel-in-progress: true`.** If you cancel a workflow mid-`docker push`,
  the manifest may be partially uploaded and the next run will see a
  half-tagged image. Keep cancellation off.
- **Adding `:latest` or `:main-latest` tags.** Stick to `:<short-sha>` and
  `:staging`. `:latest` is an anti-pattern in any environment you care about.
- **Building without the `revision` label.** Then `promote.yml` can't
  resolve `:staging` → SHA, and you'll see "Could not resolve commit SHA for
  ghcr.io/.../<service>-api:staging" errors at promote time.
- **Adding a deploy step here.** Out of scope. Put it in a sibling workflow
  in the deployer's repo (or a separate file in this repo if you really
  want — but not inside `build-api.yml`).
- **Sharing the `gha` cache scope with another service.** The two services
  evict each other; you lose ~10 minutes of build cache on every push. Use
  `scope=api` and `scope=web`.
