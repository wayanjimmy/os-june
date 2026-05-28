# `promote.yml` — re-tag staging → production (GHCR-only)

Production promotion is the load-bearing decision in this whole pipeline:
**production runs the same bytes that ran in staging**. The workflow doesn't
build anything; it verifies an image exists in GHCR and re-tags it as
`:production`. **No deploy happens here** — the deployer's tooling owns the
"actually run it somewhere" step and consumes whichever GHCR tag suits them.

## How it's invoked

The workflow only runs from `workflow_dispatch` (manual). Three inputs:

| Input | Default | Purpose |
|---|---|---|
| `artifact` | `all` | `api`, `portal`, or `all`. Which service(s) to promote. |
| `from-tag` | `staging` | Source tag to promote. Pass `staging` for normal release; pass `<short-sha>` for rollback. |

From the GitHub CLI:

```bash
# Normal release: promote whatever is currently :staging
gh workflow run promote.yml -f artifact=api

# Promote both services in one go
gh workflow run promote.yml -f artifact=all

# Rollback: promote a known-good SHA back to :production
gh workflow run promote.yml -f artifact=api -f from-tag=a1b2c3d
```

## What it does, step by step

1. **`select`** — expand the artifact input into a JSON list (`["api"]`,
   `["portal"]`, or `["api","portal"]`). The matrix uses this list so the
   downstream jobs run in parallel for `all`.
2. **`verify-images`** — for each artifact, check that
   `ghcr.io/<owner>/<service>:<from-tag>` actually exists. Fail closed if the
   tag doesn't resolve. This is what stops "I typo'd the SHA" mistakes.
3. **`retag`** — for each artifact (in parallel via matrix):
   - Resolve the source tag (`:staging` or `:<sha>`) to a short SHA by reading
     the `org.opencontainers.image.revision` label from the manifest.
   - Resolve the digest of the SHA-tagged image.
   - Re-tag the digest as `:production` using
     `docker buildx imagetools create`. This is the atomic GHCR-side pointer
     move — no bytes transferred, milliseconds of work.

## Template

See [assets/workflows/promote.yml](../assets/workflows/promote.yml). The big
shape:

```yaml
name: Promote artifact to production

on:
  workflow_dispatch:
    inputs:
      artifact:
        description: Which artifact(s) to promote.
        required: true
        type: choice
        options: [all, api, portal]
      from-tag:
        description: Source tag. Use staging for normal release or a short SHA for rollback.
        required: false
        default: staging
        type: string

permissions:
  contents: read
  packages: write          # GHCR re-tag

concurrency:
  group: promote-production
  cancel-in-progress: false

env:
  GHCR_OWNER: ${{ github.repository_owner }}
```

### `select` — turn one input into a matrix list

```yaml
jobs:
  select:
    runs-on: ubuntu-latest
    outputs:
      artifacts: ${{ steps.artifacts.outputs.artifacts }}
    steps:
      - id: artifacts
        env:
          ARTIFACT: ${{ inputs.artifact }}
        run: |
          set -euo pipefail
          case "$ARTIFACT" in
            all)        ARTIFACTS='["api","portal"]' ;;
            api|portal) ARTIFACTS="[\"$ARTIFACT\"]" ;;
            *)          echo "::error::Unknown artifact: $ARTIFACT"; exit 2 ;;
          esac
          echo "artifacts=${ARTIFACTS}" >> "$GITHUB_OUTPUT"
```

If you only have an API and no portal, drop `portal` from the case
statement and from the choices in `inputs.artifact.options`.

### `verify-images` — fail closed if the source doesn't exist

```yaml
  verify-images:
    needs: select
    runs-on: ubuntu-latest
    strategy:
      fail-fast: false
      matrix:
        artifact: ${{ fromJson(needs.select.outputs.artifacts) }}
    env:
      FROM_TAG: ${{ inputs.from-tag }}
    steps:
      - uses: docker/setup-buildx-action@v3
      - uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}

      - name: Resolve image name
        id: image
        env:
          ARTIFACT: ${{ matrix.artifact }}
        run: |
          case "$ARTIFACT" in
            api)    IMAGE="ghcr.io/${{ github.repository_owner }}/<service>-api" ;;
            portal) IMAGE="ghcr.io/${{ github.repository_owner }}/<service>-portal" ;;
          esac
          echo "name=${IMAGE}" >> "$GITHUB_OUTPUT"

      - name: Verify source image exists
        run: |
          set -euo pipefail
          image="${{ steps.image.outputs.name }}:${FROM_TAG}"
          if ! docker buildx imagetools inspect "$image" --raw > /dev/null 2>&1; then
            echo "::error::Image $image was not found in GHCR. Build that artifact/tag first, then rerun promotion."
            exit 1
          fi
          echo "Found $image"
```

`imagetools inspect --raw` is the cheapest way to confirm a manifest exists
in the registry — no pull, no extract, just a HEAD request. Side effects:

1. Typos fail fast with a clear error.
2. The "image doesn't exist yet on a brand-new service" case is caught
   before we waste effort.

### `retag` — resolve SHA, move `:production` pointer

```yaml
  retag:
    needs: [select, verify-images]
    runs-on: ubuntu-latest
    strategy:
      fail-fast: false
      matrix:
        artifact: ${{ fromJson(needs.select.outputs.artifacts) }}
    env:
      FROM_TAG: ${{ inputs.from-tag }}
    steps:
      - uses: docker/setup-buildx-action@v3
      - uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}

      - name: Resolve artifact
        id: artifact
        env:
          ARTIFACT: ${{ matrix.artifact }}
        run: |
          case "$ARTIFACT" in
            api)    IMAGE="ghcr.io/${{ github.repository_owner }}/<service>-api" ;;
            portal) IMAGE="ghcr.io/${{ github.repository_owner }}/<service>-portal" ;;
          esac
          echo "image=${IMAGE}" >> "$GITHUB_OUTPUT"

      - name: Resolve source to SHA tag
        id: image
        run: |
          set -euo pipefail
          source="${{ steps.artifact.outputs.image }}:${FROM_TAG}"
          rev=$(docker buildx imagetools inspect "$source" --format '{{json .Image}}' \
            | jq -r 'first(.. | objects | select(.config?.Labels?["org.opencontainers.image.revision"] != null) | .config.Labels["org.opencontainers.image.revision"]) // empty')

          if [[ -z "$rev" && "$FROM_TAG" =~ ^[0-9a-f]{7,40}$ ]]; then
            rev="$FROM_TAG"
          fi

          if [[ -z "$rev" ]]; then
            echo "::error::Could not resolve commit SHA for $source. Use an image built by this repository, or pass a SHA tag directly."
            exit 1
          fi

          sha_short="${rev:0:7}"
          echo "sha_short=${sha_short}" >> "$GITHUB_OUTPUT"
          echo "Resolved $source to $sha_short"

      - name: Retag SHA to production
        run: |
          set -euo pipefail
          image="${{ steps.artifact.outputs.image }}"
          sha_image="${image}:${{ steps.image.outputs.sha_short }}"
          target="${image}:production"

          digest=$(docker buildx imagetools inspect "$sha_image" \
            --format '{{ json .Manifest }}' | jq -r '.digest')
          if [[ -z "$digest" || "$digest" == "null" ]]; then
            echo "::error::Could not resolve digest for $sha_image"; exit 1
          fi

          docker buildx imagetools create --tag "$target" "${image}@${digest}"
          echo "Retagged $sha_image as $target"

      - name: Summary
        run: |
          {
            echo "### Promoted ${{ matrix.artifact }}"
            echo "- **Source tag:** \`${FROM_TAG}\`"
            echo "- **Resolved SHA:** \`${{ steps.image.outputs.sha_short }}\`"
            echo "- **Target tag:** \`${{ steps.artifact.outputs.image }}:production\`"
          } >> "$GITHUB_STEP_SUMMARY"
```

Three load-bearing pieces:

1. **`Resolve source to SHA tag`.** The `org.opencontainers.image.revision`
   label is the source of truth. The "if the from-tag *is* a SHA, use that"
   fallback handles rollback to a SHA that maybe didn't get a revision label
   (legacy images, externally-built images).
2. **`Retag SHA to production`.** `docker buildx imagetools create --tag
   target source@digest` re-points the `:production` tag at the SHA's digest
   without pulling or pushing any bytes. This is **the cheapest possible
   release-marker operation** — milliseconds, no transfer.
3. **No deploy step.** Whoever owns the deploy hop reads the new
   `:production` tag (or the resolved SHA, if they're being good citizens)
   and rolls it out on their own schedule.

## Rollback (the same workflow)

```bash
gh workflow run promote.yml -f artifact=api -f from-tag=<previous-sha>
```

Pre-conditions:

- The previous SHA tag still exists in GHCR. (Both `os-accounts` and
  `os-platform` don't currently GC GHCR tags, so this is fine for the
  foreseeable future. If you do start GC'ing, exclude the last N SHAs and
  anything ever tagged `:production`.)

The workflow:

1. Verifies `<service>:<sha>` exists.
2. Resolves `<sha>` → `<sha>` (trivially).
3. Re-tags `:production` to point at it.

Staging is *unaffected* by rollback. If you also want to roll staging back,
re-run `build-api.yml` from the previous commit (`gh workflow run
build-api.yml --ref <previous-commit>`) — or just live with staging being
ahead of production for a while.

How the rollback actually reaches the running service is, again, the
deployer's job. They'll see `:production` move and roll forward (which is
backward, in this case).

## Common `promote.yml` mistakes

- **Skipping `verify-images`.** A failed deploy with a missing source image
  is a much worse failure mode (cryptic errors downstream) than a failed
  verification step (clear "image not found" up front).
- **Tagging `:production` *before* resolving the SHA.** Then a typo in
  `from-tag` leaves `:production` pointing somewhere wrong while you don't
  notice. Always resolve and verify first; retag last.
- **Adding a deploy step here.** Out of scope. Build a separate workflow
  triggered by the `:production` tag movement (or just have your deployer
  poll / get pinged).
- **Re-running promotion expecting "promote to next-best image".** It
  promotes the image you tell it to. There is no automatic "use whatever's
  latest" logic; that would be surprising.
- **Treating `:production` as a deployment target.** It's a GHCR tag — a
  *marker*. Whether something runs from it is up to whatever consumes the
  registry.
