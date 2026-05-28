---
name: os-rust-backend
description: >-
  Use when working on a Rust backend that follows the Open Software Network
  house style — the seven-crate Cargo workspace split (`domain / services /
  persistence / providers / config / api / app`) shared by `os-accounts` and
  `os-platform` (`fellow`). Trigger on: scaffolding a new service in this org
  ("like os-accounts", "fellow-sidekick"), deciding **which crate something
  belongs in** (trait vs impl, sqlx pool, reqwest call, business logic,
  handler), adding an endpoint returning the `ApiResponse<T>` envelope, wiring
  a provider behind a `#[async_trait]` trait, writing sqlx repositories or
  migrations with snake_case enums and `CHECK` constraints, or integration
  tests with `rstest` + real Postgres + `wiremock`. Signals without naming the
  architecture: `accounts-*` / `fellow-*` / `os-*` crate prefixes, axum + sqlx
  + figment in this org, `osk_` / `usr_` ids, figment-typed config (no
  `env::var`), or questions like "where does this live" / "how do we shape
  backends here".
---

# Open Software backend (Rust)

This skill captures the **house style** used by `os-accounts` and `os-platform`
(the `fellow` codebase). Both repositories were built independently and
converged on the same shape — that convergence is not coincidence, it is the
recipe. Follow it when starting a new service or extending one of these two, and
your code will look at home in either repo.

The recipe in one sentence: **a Cargo workspace of small, pointed crates where
the domain doesn't know about HTTP, services don't know about Postgres, the API
doesn't know about Privy or Stripe, and the binary wires it all together.**

Read this file, then open the reference that matches the work.

## Route the request first

| If the user asks for... | Open first | Non-negotiable |
|---|---|---|
| "scaffold a new backend" / "start a service like os-accounts" | [workspace-layout.md](references/workspace-layout.md) | Seven crates, exact names: `domain / services / persistence / providers / config / api / app`. |
| "add an endpoint" / "what should this handler return" | [api-envelope.md](references/api-envelope.md) | Every endpoint returns `ApiResponse<T>`. Error code bands map to HTTP status. |
| "where do I read `DATABASE_URL`" / "add a new config knob" | [config-and-providers.md](references/config-and-providers.md) | Never `std::env::var`. Add a typed field to `*-config`, plumb it down. |
| "wire up Stripe / Privy / a new external API" | [config-and-providers.md](references/config-and-providers.md) | Define a trait in `*-domain`, implement in `*-providers`, inject `Arc<dyn Trait>` in `app/main.rs`. |
| "add a table / write a query / run a migration" | [persistence.md](references/persistence.md) | sqlx + `MIGRATIONS!`. Repository structs in `*-persistence`. Enum columns get `CHECK` constraints. |
| "test the new endpoint" / "how do I mock Privy" | [testing.md](references/testing.md) | Unit = no I/O; integration = real Postgres + wiremock; e2e = HTTP against a deployed service. |
| "containerize it for GHCR" | [dockerfile.md](references/dockerfile.md) | cargo-chef multi-stage, debian-slim runtime, non-root user, `ENTRYPOINT` is the binary. |

## Mental model: the dependency cone

```
                              app/main.rs
                              │ (only the binary knows everyone)
        ┌─────────────────────┼─────────────────────┐
        ▼                     ▼                     ▼
     accounts-api      accounts-services     accounts-providers
        │                  │                       │
        └────────┬─────────┴──────────┬────────────┘
                 ▼                    ▼
          accounts-persistence    accounts-domain  ◀── traits live here
                 │                    ▲
                 └────────────────────┘
                          ▲
                 accounts-config (shared types only)
```

Read the arrows as "depends on". The shape gives you four properties:

1. **`domain` is pure.** No HTTP, no DB, no `reqwest`. It defines value types, IDs,
   errors, and **the traits** (`IdentityProvider`, `PaymentGateway`, …) — the
   ports the rest of the world plugs into.
2. **`persistence` and `providers` know nothing about each other.** Both depend
   on `domain` so they can implement domain traits and return domain types, but
   they don't talk through anything except `domain`. This means you can rewrite
   Stripe → Adyen without touching the database layer, and you can swap Postgres
   → CockroachDB without touching Privy.
3. **`services` is the orchestrator.** This is where business logic lives — the
   things that read from `persistence`, call `providers`, return domain results.
   The API layer should be thin; if a handler has branching business logic, that
   logic belongs in `services`.
4. **`app/main.rs` is the only place that names concrete provider impls.**
   `accounts_providers::ConfiguredIdentityProvider::from_config(...)`, then
   `Arc::new(...) as Arc<dyn IdentityProvider>` into the router state. The rest
   of the code only sees the trait.

If you find yourself adding `sqlx` to `accounts-providers`, or `reqwest` to
`accounts-persistence`, or `axum` to `accounts-domain` — **stop**. The shape is
wrong; the change probably belongs in a different crate.

## Golden rules (and why they matter)

1. **One service-prefix, every crate.** `accounts-api`, `accounts-domain`, etc.
   for an "accounts" service; `fellow-api`, `fellow-domain`, etc. for the
   `os-platform` service. The prefix is the service identity — it is how a
   reader recognizes which workspace they are in.
2. **The binary crate's package name is the bare service noun.** `accounts`,
   `fellow`. The library crates carry the prefix; the binary doesn't, because
   `cargo install accounts` reads more naturally than `cargo install
   accounts-app`.
3. **Edition 2024 + pinned toolchain.** The toolchain channel lives in
   `rust-toolchain.toml`; the MSRV (`rust-version`) lives in the workspace
   `[workspace.package]` in `Cargo.toml` and is mirrored by `clippy.toml`.
   Both repos pin `1.95.0` and let CI install exactly that. Don't let the
   toolchain float.
4. **Workspace deps, not crate-local versions.** Every external dep is declared
   in the root `[workspace.dependencies]` once. Member crates write
   `serde.workspace = true`. No version skew across crates, no diamond
   problems.
5. **Workspace lints are pedantic + paranoid.** `clippy::pedantic` at warn,
   `unsafe_code = forbid`, `unwrap_used` / `expect_used` / `panic` / `todo` /
   `unimplemented` / `dbg_macro` / `print_stdout` / `print_stderr` all at warn,
   `-D warnings` in CI. This is what keeps the codebase out of "Rust slop"
   territory — no `.unwrap()` sneaks in, no `dbg!` ships.
6. **`std::env::var` is a `clippy.toml` disallowed-method.** See [spec/rust-config-over-env.md](../../spec/rust-config-over-env.md).
   Add the env knob to your `*Config` struct and let figment merge it.
7. **Every endpoint returns `ApiResponse<T>`.** No raw arrays at the top level,
   no `Json<MyThing>` returned directly. See [api-envelope.md](references/api-envelope.md).
8. **Error codes are application-level integers, banded by HTTP status.**
   `1001–1999 → 404`, `2001–2999 → 400`, `3001–3999 → 403`, `4001–4099 → 409`,
   `4101–4199 → 410`, `4201–4299 → 422`, `4301–4399 → 402`, `4401–4499 → 429`,
   `5000–5099 → 500`. The envelope sets the HTTP status from the band so
   clients can branch on either status or `error_code`.
9. **`tracing` only — no `println!` / `eprintln!` / `dbg!`.** Use structured
   fields: `tracing::error!(%err, "connection failed")`, not
   `tracing::error!("connection failed: {}", err)`. The JSON formatter relies
   on the fields being typed.
10. **Wire enums use lowercase / snake_case in both JSON and SQL.** Rust enums
    stay PascalCase; `#[serde(rename_all = "snake_case")]` and `#[sqlx(rename_all
    = "snake_case")]` translate at the boundary. Every enum column gets a
    SQL `CHECK` constraint enumerating the allowed values — never an open
    `TEXT` column. See [persistence.md](references/persistence.md).
11. **Functions with 3+ parameters take a `*Params` struct.** Call sites read
    like documentation, adding a field is non-breaking. The repos already enforce
    this — see [spec/struct-params.md](../../spec/struct-params.md).
12. **No I/O in unit tests.** If your test needs Postgres, it's an integration
    test and lives in `crates/<x>/tests/`. Mock providers with `wiremock`, not
    hand-rolled fakes inside `#[cfg(test)]`.

## Scaffold checklist (new service)

If the user is starting a brand-new service in this style (not extending
`os-accounts` or `os-platform`):

1. **Pick the service-prefix.** A short noun, lowercase, no hyphens: `accounts`,
   `fellow`, `library`, `forum`. Used as `<prefix>-api`, `<prefix>-domain`, etc.,
   and as the binary's package name (bare, no prefix).
2. **Create the workspace.** Root `Cargo.toml` with `resolver = "3"`,
   `members = ["crates/*"]`, the workspace-package fields, the full
   `[workspace.dependencies]` block (copy from `api/Cargo.toml` in either repo
   and trim deps you don't need), and the workspace lints block. See
   [workspace-layout.md](references/workspace-layout.md) for the exact template.
3. **Create the seven crates** under `crates/`: `domain`, `services`,
   `persistence`, `providers`, `config`, `api`, `app`. Only `app` is a `[[bin]]`;
   the others are libraries. Each crate's `Cargo.toml` is small — it inherits
   nearly everything from the workspace.
4. **Add `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`** at the workspace
   root. Pin the toolchain (`1.95.0` is the current default), turn on the same
   rustfmt knobs (`edition = "2024"`, `max_width = 100`, `hard_tabs = false`,
   `tab_spaces = 4`), and list the disallowed env methods in `clippy.toml`.
5. **Define the first domain type and the first trait.** Don't start with the
   handler. Start with `domain/src/lib.rs`: declare the `User` (or whatever your
   first entity is), declare the first port (`IdentityProvider` etc.) as a
   `#[async_trait]` trait, declare a `DomainError` enum with `thiserror`.
6. **Build `*-config` second.** A single root struct (`AccountsConfig`,
   `FellowConfig`) with nested sections (`server`, `database`, ...), a
   `pub fn load() -> Result<Self, ConfigError>` that uses figment to merge
   defaults + `config.toml` + env, and a small test that loads it from a
   fixture.
7. **Add the `app` binary stub.** Parse `Cli` with `clap` (subcommands `Serve`,
   `Migrate`), call `accounts_config::load()`, init `tracing_subscriber` with a
   JSON formatter, wire `axum::serve` against a router from `accounts-api`. The
   binary is the only crate that depends on every other crate.
8. **Add the `/livez` health endpoint and the `ApiResponse` type next.** Get
   the envelope shape right *before* writing real endpoints — every handler
   will use it.
9. **Iterate.** Real endpoints, real tables, real providers — but always in the
   same crate boundaries. Each new endpoint: handler in `api`, business logic in
   `services`, queries in `persistence`, external calls in `providers`, types
   in `domain`.

## Workspace deps quick-reference

These are the libraries every Open Software backend uses. Reach for these first
before introducing alternatives — consistency is part of the recipe.

| Concern | Crate(s) | Notes |
|---|---|---|
| Async runtime | `tokio` (`"full"` features) | One runtime, full features in the workspace. |
| HTTP server | `axum`, `tower`, `tower-http` | `tower-http` features: `trace`, `cors`, `timeout`, `limit`. |
| HTTP client | `reqwest` (`"json"`) | Only inside `*-providers`. |
| Serialization | `serde`, `serde_json`, `base64`, `hex` | `serde` always with `derive`. |
| Errors | `thiserror` (libraries), `anyhow` (binary only) | Library crates never return `anyhow::Error`. |
| Logging | `tracing`, `tracing-subscriber` (`env-filter`, `json`) | JSON formatter; `EnvFilter::try_from_default_env()`. |
| Database | `sqlx` (`postgres`, `uuid`, `chrono`, `migrate`, `macros`, `bigdecimal`, `json`) | Only inside `*-persistence` and `app`. |
| Money | `bigdecimal` (`serde`) | Never `f64` for money. Prefer integer credits / cents at the wire — see [api-envelope.md](references/api-envelope.md). |
| Config | `figment` (`toml`, `env`), `dotenvy` | `dotenvy` only in `app`. |
| OpenAPI | `utoipa`, `utoipa-axum`, `utoipa-scalar` | Derive `ToSchema` on every wire DTO. |
| CLI | `clap` (`derive`) | Only inside `app`. |
| IDs | `uuid` (`v4`, `v7`, `serde`), `nanoid` | `v7` for new ids; `nanoid` for opaque short ids. |
| Time | `chrono` (`serde`) | Persist `TIMESTAMPTZ`, serialize as RFC3339. |
| Auth/crypto | `jsonwebtoken`, `hmac`, `sha2`, `subtle` | `subtle::ConstantTimeEq` for any secret compare. |
| Async traits | `async-trait` | On every domain trait that has async methods. |
| URL parsing | `url` | At trust boundaries (redirects, webhooks). |
| Testing | `mockall`, `wiremock`, `rstest`, `pretty_assertions`, `insta` | See [testing.md](references/testing.md). |

## Common mistakes to avoid

- **Cramming everything into one crate.** "It's a small service, I'll just have
  a `src/` directory." The crate split *is* the architectural review tool — you
  cannot accidentally use Stripe from inside a SQL query if `*-persistence`
  doesn't depend on `*-providers`. Don't skip it because it feels heavy at
  scale 1; you'll regret it at scale 5.
- **Putting trait impls in the wrong crate.** `impl IdentityProvider for
  PrivyClient` lives in `*-providers`, not in `*-domain`. The trait lives in
  `domain`; impls live with the technology they wrap.
- **Returning `Json<T>` directly from a handler.** That ships a raw payload and
  breaks every caller's `success` check. Always wrap in `ApiResponse`.
- **Reading `std::env::var` inside a handler or service.** Every config knob
  belongs in the `*Config` struct and flows in via `ApiState`. The disallowed-method
  lint will catch you in CI; don't paper over it with `#[allow(...)]`.
- **`tracing::info!("user {} logged in", user_id)` instead of
  `tracing::info!(%user_id, "user logged in")`.** The first is a string —
  the JSON formatter can't index it. Use structured fields.
- **Logging a secret.** Action-token bearers (`agts_…`), refresh tokens, the
  App API key (`osk_…`), JWTs — none of these go in any log line, span attribute,
  error message, or response body. Types that hold a secret implement `Debug`
  manually and redact the field; never `?token` in a `tracing::*!` call.
- **Open `TEXT` enum columns.** Add a `CHECK` constraint enumerating the values,
  keep the SQL value and the JSON wire value identical (lowercase /
  snake_case), and have the Rust enum translate via `#[serde(rename_all)]`.
- **One mega `errors.rs` shared by every crate.** Each layer defines its own
  error enum (`DomainError`, `PersistenceError`, `ProviderError`,
  `ServiceError`) and converts at boundaries with `#[from]`. The API layer
  maps the top-level service error into an `ApiResponse` with the right error
  code.
- **`Arc<Mutex<DbPool>>` or other "I'll just clone the world into state".**
  `sqlx::PgPool` is already cheap to clone. `Arc<dyn Trait>` is the right shape
  for shared trait objects. Don't reach for `Mutex` unless you're guarding
  mutable shared in-memory state (rare — the rate-limiter is the only example
  in `os-accounts`, and it's bounded).
- **Skipping integration tests because "the unit tests pass".** The unit tests
  prove the pure logic; the integration tests prove the SQL is real, the
  migrations apply, the route is wired, the JSON shape is the envelope. You
  need both. See [testing.md](references/testing.md).
- **Building a Docker image that runs as root or includes the build toolchain.**
  Multi-stage with cargo-chef; runtime is `debian:13-slim` (or matching);
  `useradd` a non-root user and `USER` to it; copy only the release binary.
  See [dockerfile.md](references/dockerfile.md).

## After scaffolding: where to look next

- For **wiring CI** (format, clippy, tests, contract drift, GHCR image build,
  artifact promotion via re-tag), use the sibling skill **`os-rust-backend-ci`**.
  It ships templated workflows and composite actions that match this recipe.
  *Deployment* (how artifacts get to a running environment) is out of scope
  for both skills — that's owned by whoever runs the service.
- For **integrating apps that depend on this backend** (Login with Open Software,
  metering, etc.), see the **`os-accounts-integration`** skill.
- For **the actual deployed topology** (domains, BFF), see
  `docs/deployment.md` in either repo.
- For **the live decisions** that shaped this recipe, read `docs/adr/` in
  either repo — particularly ADR-0001 (platform shape).
