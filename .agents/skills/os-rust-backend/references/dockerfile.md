# Dockerfile

Both repos use the same Dockerfile shape: **cargo-chef** for dependency caching,
a fat builder stage with the toolchain, a thin runtime stage with just glibc +
TLS roots + the release binary. The image runs as a non-root user. This is
what's used in CI to publish to GHCR.

## The Dockerfile

Drop this at `api/Dockerfile`:

```dockerfile
# syntax=docker/dockerfile:1.24.0

# --- planner -----------------------------------------------------------------
FROM lukemathwalker/cargo-chef:0.1.77-rust-1.95.0-slim-trixie AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# --- builder -----------------------------------------------------------------
FROM chef AS builder
RUN apt-get update \
  && apt-get install -y --no-install-recommends clang libssl-dev mold pkg-config \
  && rm -rf /var/lib/apt/lists/*

COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

COPY . .
RUN cargo build --release --bin library    # <- replace `library` with your binary name

# --- runtime -----------------------------------------------------------------
FROM debian:13.5-slim AS runtime
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates libssl3t64 \
  && rm -rf /var/lib/apt/lists/* \
  && useradd --uid 10001 --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin library

WORKDIR /app
COPY --from=builder /app/target/release/library /usr/local/bin/library

ENV LIBRARY_SERVER__HOST=0.0.0.0
USER library
ENTRYPOINT ["/usr/local/bin/library"]
```

Three things to swap when copying to a new service:

1. Replace `library` (binary name) with your service noun (`accounts`, `fellow`,
   …) in three places: the `cargo build --bin`, the `COPY --from=builder`
   target path's basename, the `useradd ... library`, the `USER library`, and
   the `ENTRYPOINT`.
2. Replace `LIBRARY_SERVER__HOST` with your config-env prefix (the figment
   `Env::prefixed("LIBRARY_")` value).
3. Bump the Rust version on both the `cargo-chef` tag and `api/rust-toolchain.toml`
   together — they must match, or the cached builder image will compile with
   the wrong compiler.

## Why this shape

- **cargo-chef** computes a "recipe" (the dep graph) and pre-builds just the
  deps, separately from the workspace source. The result: a docker layer
  cache where dep compile times survive every source change. Without chef,
  every CI run rebuilds 800 crates of dependencies.
- **mold** as the linker. mold is multiple-x faster than ld for big Rust
  projects. It's installed in the builder; `cargo` picks it up automatically
  if a `.cargo/config.toml` declares it. Or set it via env:
  ```dockerfile
  ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=clang
  ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-C link-arg=-fuse-ld=/usr/bin/mold"
  ```
  Both repos rely on workspace defaults — uncomment the env lines if your local
  builds aren't picking up mold.
- **debian:13-slim (trixie)** as the runtime. Slim base, modern glibc, modern
  OpenSSL 3 (`libssl3t64`). Distroless is tempting, but you give up debugging
  ergonomics (no shell to drop into) and gain little — slim is already ~80MB.
- **Non-root user.** `useradd --uid 10001 --no-create-home`. Most platforms
  will warn or refuse to run root containers.
- **`ENTRYPOINT` is the binary**, not `CMD`. `ENTRYPOINT` makes the binary
  unconfusable for users running `docker run`, and lets the runtime append
  the subcommand (`serve`, `migrate`) as the container's args.

## Building locally

The Dockerfile builds with the `api/` workspace as its context — the cargo-chef
planner copies *everything* in that directory tree, so make sure your
`.dockerignore` excludes `target/`:

```
# api/.dockerignore
target/
.git/
.idea/
.vscode/
**/*.swp
**/.DS_Store
```

```bash
docker build --file api/Dockerfile --tag library:test api
docker run --rm -p 8080:8080 \
    -e LIBRARY_DATABASE__URL='postgres://host.docker.internal:5432/library' \
    library:test serve
```

If your dev environment uses a Docker credential helper that doesn't work in
your current shell, point Docker at a throwaway config dir:

```bash
DOCKER_CONFIG=/private/tmp/library-docker-config \
    docker build --file api/Dockerfile --tag library:test api
```

## Image labels

The build workflow stamps OpenContainers labels on the pushed image:

```yaml
labels: |
  org.opencontainers.image.source=https://github.com/${{ github.repository }}
  org.opencontainers.image.revision=${{ github.sha }}
  org.opencontainers.image.created=${{ github.event.head_commit.timestamp }}
```

The `revision` label is what `promote.yml` reads when resolving `:staging` →
`:<short-sha>`. Don't strip these in the Dockerfile.

## Migration sub-command

Because `ENTRYPOINT = /usr/local/bin/library`, the runtime can invoke
migrations with `/usr/local/bin/library migrate` before the serving release
starts. The container starts, applies migrations, exits, then the new
release boots and serves traffic. No separate migration job, no
half-migrated database under partial deploys.

A typical pre-deploy / health-check shape consumed by whatever runtime you
deploy to:

```json
{
  "deploy": {
    "preDeployCommand": "/usr/local/bin/library migrate",
    "healthcheckPath": "/livez",
    "healthcheckTimeout": 30,
    "restartPolicyType": "ON_FAILURE",
    "restartPolicyMaxRetries": 10,
    "numReplicas": 1
  }
}
```

## Size & cache notes

- Final image lands around **90–120 MB** for a small service; ~150 MB for one
  with sqlx + reqwest + opentelemetry. If it's much bigger, you've leaked a
  build dep into the runtime layer.
- GitHub Actions cache scopes the layer cache per service (`scope=api`).
  Without scoping, separate workflows fight over the same cache key and evict
  each other.
- `cargo-chef` is the same version across both repos (0.1.77 at time of
  writing). Bump it together with the Rust version.

## Common Dockerfile mistakes

- **`COPY target/release/...` from the host.** The whole point of the multi-stage
  build is that the host doesn't need to compile. Always copy from `--from=builder`.
- **Forgetting the non-root `USER`.** Many platforms hard-reject root containers.
- **Hardcoding `0.0.0.0` in source instead of `ENV LIBRARY_SERVER__HOST=0.0.0.0`.**
  The env approach keeps dev (`127.0.0.1`) and prod (`0.0.0.0`) distinct without
  source changes.
- **Apt-installing `openssl` in the runtime stage.** You want `libssl3t64`
  (the runtime library), not the dev package. The dev package is for compilers.
- **`apt-get update` without `rm -rf /var/lib/apt/lists/*`.** Bloats the image
  with package indexes the runtime never uses.
- **Using `:latest` as the cargo-chef tag.** The base image moves under you;
  builds become non-reproducible. Pin to a specific tag.
