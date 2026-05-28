# Workspace layout

The seven-crate split is the architectural review tool. Get this right first;
everything else follows.

## Directory shape

```
api/                                # the workspace lives under api/ so the repo
                                    # has room for web/, example/, iac/, etc.
├── Cargo.toml                      # workspace root
├── Cargo.lock                      # committed
├── rust-toolchain.toml             # pin the channel
├── rustfmt.toml                    # formatting knobs
├── clippy.toml                     # disallowed methods, thresholds
├── Dockerfile                      # cargo-chef multi-stage
├── openapi.json                    # generated from utoipa, checked in
├── migrations/                     # sqlx migrations: NNNN_name.sql
└── crates/
    ├── domain/
    │   └── src/
    │       ├── lib.rs              # value types, IDs, traits, errors
    │       └── scope.rs            # OAuth scope union — example of a tight sub-module
    ├── services/
    │   └── src/
    │       ├── lib.rs              # re-exports
    │       ├── auth.rs             # token issuer, PKCE matching
    │       ├── grants.rs           # /authorize -> /charge business logic
    │       └── action_token.rs     # mint + verify
    ├── persistence/
    │   └── src/
    │       └── lib.rs              # PgPool, repositories, query structs
    ├── providers/
    │   └── src/
    │       └── lib.rs              # ConfiguredIdentityProvider, ConfiguredPaymentGateway
    ├── config/
    │   └── src/
    │       └── lib.rs              # AccountsConfig + figment loader
    ├── api/
    │   └── src/
    │       ├── lib.rs              # ApiState, router(), handlers
    │       └── bin/                # cargo binaries used in dev only (openapi export, etc.)
    └── app/
        └── src/
            └── main.rs             # tokio::main, dotenvy, clap, axum::serve
```

## Crate purposes — read this before adding code anywhere

| Crate | Owns | Doesn't know about |
|---|---|---|
| `*-domain` | Value types, IDs (newtypes), domain errors, **traits** (ports). | HTTP, SQL, `reqwest`, `axum`. Pure logic only. |
| `*-services` | Business logic. Orchestrates domain + persistence + providers. | `axum::Json`, `tower`, HTTP status codes, request DTOs. |
| `*-persistence` | `sqlx::PgPool`, repository functions, row structs, `*Params` query inputs. | `reqwest`, `axum`, anything HTTP. |
| `*-providers` | External-API clients implementing domain traits (Privy, Stripe, JWKS, …). | `sqlx`, `axum`, the database schema. |
| `*-config` | `Config` struct + `load()` returning it. Typed access to every knob. | Anything except `figment`, `serde`, `thiserror`. |
| `*-api` | Axum router, handlers, request/response DTOs, `ApiResponse` envelope, OpenAPI annotations. | `dotenvy`, `clap`, choosing which provider impl to instantiate. |
| `app` (bin) | `tokio::main`, `dotenvy`, `clap`, logging init, `tracing_subscriber` setup, **instantiating provider impls and constructing `ApiState`**. | Business logic. The binary should be thin and unsurprising. |

## Workspace root `Cargo.toml` template

This is the minimal viable root for a new service called `library` (substitute
your own prefix). It mirrors what `os-accounts` and `os-platform` use today.

```toml
[workspace]
resolver = "3"
members = ["crates/*"]

[workspace.package]
edition = "2024"
rust-version = "1.95"
license = "UNLICENSED"
authors = ["Open Software"]
publish = false

[workspace.dependencies]
# --- Internal crates ---
library-api = { path = "crates/api" }
library-config = { path = "crates/config" }
library-domain = { path = "crates/domain" }
library-services = { path = "crates/services" }
library-persistence = { path = "crates/persistence" }
library-providers = { path = "crates/providers" }

# --- Async runtime ---
tokio = { version = "1", features = ["full"] }

# --- HTTP server ---
axum = "0.8"
tower = { version = "0.5", features = ["util"] }
tower-http = { version = "0.6", features = ["trace", "cors", "timeout", "limit"] }

# --- HTTP client ---
reqwest = { version = "0.12", features = ["json"] }

# --- Serialization ---
serde = { version = "1", features = ["derive"] }
serde_json = "1"
base64 = "0.22"
hex = "0.4"

# --- Errors ---
thiserror = "2"
anyhow = "1"

# --- Observability ---
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }

# --- Database ---
sqlx = { version = "0.8", features = [
  "runtime-tokio", "postgres", "uuid", "chrono", "migrate", "macros", "bigdecimal", "json",
] }
bigdecimal = { version = "0.4", features = ["serde"] }

# --- Config ---
figment = { version = "0.10", features = ["toml", "env"] }
dotenvy = "0.15"

# --- OpenAPI ---
utoipa = { version = "5", features = ["axum_extras", "chrono", "uuid"] }
utoipa-axum = "0.2"
utoipa-scalar = { version = "0.3", features = ["axum"] }

# --- Utilities ---
clap = { version = "4", features = ["derive"] }
uuid = { version = "1", features = ["v4", "v7", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
async-trait = "0.1"
nanoid = "0.4"
url = "2"

# --- Auth / crypto ---
jsonwebtoken = "9"
hmac = "0.12"
sha2 = "0.10"
subtle = "2.6"

# --- Testing ---
mockall = "0.13"
wiremock = "0.6"
rstest = "0.23"
pretty_assertions = "1"
insta = { version = "1", features = ["json", "yaml"] }

[workspace.lints.rust]
unsafe_code = "forbid"
rust_2018_idioms = { level = "warn", priority = -1 }
unreachable_pub = "warn"
unused_lifetimes = "warn"

[workspace.lints.clippy]
pedantic = { level = "warn", priority = -1 }
unwrap_used = "warn"
expect_used = "warn"
panic = "warn"
todo = "warn"
unimplemented = "warn"
dbg_macro = "warn"
print_stdout = "warn"
print_stderr = "warn"
module_name_repetitions = "allow"
missing_errors_doc = "allow"
missing_panics_doc = "allow"
must_use_candidate = "allow"

[profile.release]
lto = "thin"
codegen-units = 1
strip = true
panic = "abort"
opt-level = 3

[profile.dev]
opt-level = 0
incremental = true
```

The `library-app` member is deliberately not in `[workspace.dependencies]` — the
binary doesn't need to be addressable as a dep, only as a build target.

## Per-crate `Cargo.toml` templates

Every library member follows the same shell — small, mostly `workspace = true`
references. Replace `library-X` with your service-prefix and prune deps you
don't need.

### `crates/domain/Cargo.toml`

```toml
[package]
name = "library-domain"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
publish.workspace = true

[lints]
workspace = true

[dependencies]
async-trait.workspace = true
chrono.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true

[dev-dependencies]
pretty_assertions.workspace = true
rstest.workspace = true
```

### `crates/services/Cargo.toml`

```toml
[package]
name = "library-services"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
publish.workspace = true

[lints]
workspace = true

[dependencies]
library-config.workspace = true
library-domain.workspace = true
library-persistence.workspace = true
async-trait.workspace = true
base64.workspace = true
chrono.workspace = true
jsonwebtoken.workspace = true
nanoid.workspace = true
serde.workspace = true
serde_json.workspace = true
sha2.workspace = true
sqlx.workspace = true
subtle.workspace = true
thiserror.workspace = true
uuid.workspace = true

[dev-dependencies]
pretty_assertions.workspace = true
tokio.workspace = true
```

### `crates/persistence/Cargo.toml`

```toml
[package]
name = "library-persistence"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
publish.workspace = true

[lints]
workspace = true

[dependencies]
library-config.workspace = true
library-domain.workspace = true
chrono.workspace = true
nanoid.workspace = true
serde.workspace = true
serde_json.workspace = true
sha2.workspace = true
sqlx.workspace = true
subtle.workspace = true
thiserror.workspace = true
```

### `crates/providers/Cargo.toml`

```toml
[package]
name = "library-providers"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
publish.workspace = true

[lints]
workspace = true

[dependencies]
library-config.workspace = true
library-domain.workspace = true
async-trait.workspace = true
chrono.workspace = true
hex.workspace = true
hmac.workspace = true
jsonwebtoken.workspace = true
reqwest.workspace = true
serde.workspace = true
serde_json.workspace = true
sha2.workspace = true
subtle.workspace = true

[dev-dependencies]
anyhow.workspace = true
pretty_assertions.workspace = true
tokio.workspace = true
wiremock.workspace = true
```

### `crates/config/Cargo.toml`

```toml
[package]
name = "library-config"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
publish.workspace = true

[lints]
workspace = true

[dependencies]
figment.workspace = true
serde.workspace = true
thiserror.workspace = true
```

### `crates/api/Cargo.toml`

```toml
[package]
name = "library-api"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
publish.workspace = true

[lints]
workspace = true

[dependencies]
library-config.workspace = true
library-domain.workspace = true
library-persistence.workspace = true
library-services.workspace = true
async-trait.workspace = true
axum.workspace = true
chrono.workspace = true
serde.workspace = true
serde_json.workspace = true
sqlx.workspace = true
thiserror.workspace = true
tower-http.workspace = true
tracing.workspace = true
url.workspace = true
utoipa.workspace = true
uuid.workspace = true

[dev-dependencies]
library-domain.workspace = true
library-persistence.workspace = true
anyhow.workspace = true
base64.workspace = true
pretty_assertions.workspace = true
sha2.workspace = true
sqlx.workspace = true
tokio.workspace = true
tower.workspace = true
```

### `crates/app/Cargo.toml`

```toml
[package]
name = "library"   # the bare service noun — no -app suffix
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
publish = false

[[bin]]
name = "library"
path = "src/main.rs"

[lints]
workspace = true

[dependencies]
library-api.workspace = true
library-config.workspace = true
library-persistence.workspace = true
library-providers.workspace = true
library-services.workspace = true
anyhow.workspace = true
axum.workspace = true
clap.workspace = true
dotenvy.workspace = true
sqlx.workspace = true
tokio.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
```

The binary is the only crate that touches **every** library crate, **`clap`**,
**`dotenvy`**, **`anyhow`**, and **`tracing-subscriber`**. If you find yourself
wanting `clap` inside `*-services`, the design is wrong — pass parsed values in.

## Toolchain pins

### `api/rust-toolchain.toml`

```toml
[toolchain]
channel = "1.95.0"
components = ["rustfmt", "clippy", "rust-src", "rust-analyzer"]
profile = "minimal"
```

Updating Rust is a deliberate act — bump the channel here, fix any new clippy
warnings in the same PR, and CI will pick up the new toolchain automatically
(the `setup-rust` composite action reads this file).

### `api/rustfmt.toml`

```toml
edition = "2024"
style_edition = "2024"
max_width = 100
hard_tabs = false
tab_spaces = 4
newline_style = "Unix"
use_field_init_shorthand = true
use_try_shorthand = true
```

### `api/clippy.toml`

```toml
cognitive-complexity-threshold = 30
too-many-arguments-threshold = 8
type-complexity-threshold = 250
trivial-copy-size-limit = 64
avoid-breaking-exported-api = false
msrv = "1.95"
disallowed-methods = [
  { path = "std::env::var", reason = "use library-config typed config instead" },
  { path = "std::env::var_os", reason = "use library-config typed config instead" },
  { path = "std::env::set_var", reason = "unsound in multithreaded contexts" },
  { path = "std::env::remove_var", reason = "unsound in multithreaded contexts" },
]
```

The disallowed-methods list is what enforces *config over env*. If a refactor
genuinely needs to read an env var (it shouldn't, but for a one-off CLI flag in
the binary it might), allow it at the call site with
`#[allow(clippy::disallowed_methods)]` and a comment explaining why — not by
removing the lint.

## The binary's `main.rs` shape

The binary should be boring and readable end-to-end. This template lines up with
how both repos do it:

```rust
//! Application binary: loads config, mounts the HTTP router, and serves.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use clap::{Parser, Subcommand};
use library_config::LibraryConfig;
use tracing_subscriber::{EnvFilter, fmt};

#[derive(Debug, Parser)]
#[command(name = "library", about = "Library service")]
struct Cli {
    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// Start the HTTP API server.
    Serve,
    /// Run database migrations and exit.
    Migrate,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    load_dotenv().context("load .env")?;
    init_tracing();

    let cli = Cli::parse();
    let config = library_config::load().context("load library config")?;

    match cli.command.unwrap_or(CliCommand::Serve) {
        CliCommand::Serve => serve(config).await,
        CliCommand::Migrate => migrate(&config).await,
    }
}

async fn serve(config: LibraryConfig) -> anyhow::Result<()> {
    let database = library_persistence::connect(&config.database).await.context("connect Postgres")?;
    library_persistence::run_migrations(&database).await.context("run migrations")?;

    let bind_addr: SocketAddr = format!("{}:{}", config.server.host, config.server.port)
        .parse()
        .context("parse bind address")?;

    let identity_provider = Arc::new(library_providers::Privy::from_config(&config.privy));
    let payment_gateway = Arc::new(library_providers::Stripe::from_config(&config.stripe));

    let state = library_api::ApiState::new(config, database, identity_provider, payment_gateway);
    let router = library_api::router(state);
    let listener = tokio::net::TcpListener::bind(bind_addr).await
        .with_context(|| format!("bind listener at {bind_addr}"))?;

    tracing::info!(%bind_addr, "starting library");
    axum::serve(listener, router).await.context("serve library")?;
    Ok(())
}

async fn migrate(config: &LibraryConfig) -> anyhow::Result<()> {
    let database = library_persistence::connect(&config.database).await.context("connect Postgres")?;
    library_persistence::run_migrations(&database).await.context("run migrations")?;
    tracing::info!("migrations complete");
    Ok(())
}

fn load_dotenv() -> anyhow::Result<()> {
    match dotenvy::dotenv() {
        Ok(_) => Ok(()),
        Err(dotenvy::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    fmt().with_env_filter(filter).json().init();
}
```

Why `Migrate` is its own subcommand: the runtime can invoke
`/usr/local/bin/library migrate` as a pre-deploy step before the serving
release starts, so migrations are guaranteed to apply before any request
hits the new release.

## When the recipe needs an eighth crate

Both repos have stayed at seven crates for a long time on purpose. Reach for an
eighth crate only when one of these is true:

- **Observability.** `os-platform` (fellow) adds `observability/` for the
  OpenTelemetry layer (OTLP exporters, span builders). This stays out of `api/`
  because the OTel deps are heavy and you want to test the layer in isolation.
- **Domain-specific subdomains too big to live in `*-services`.** If `services`
  reaches ~5 files and they cluster into orthogonal groups (auth vs. billing vs.
  catalog), promote one to its own crate. Keep the trait/contract split: the
  new crate depends on `*-domain`, not the other way round.
- **A shared client library** used by other services in the org. This crate
  re-exports the wire DTOs from `*-api` and a typed HTTP client; consumers
  depend on it instead of duplicating types.

Resist any other split. "I'll make a `*-errors` crate so every layer has a
common error type" — no, that's how layers leak. Each layer has its own error.
