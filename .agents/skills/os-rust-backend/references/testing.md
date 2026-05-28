# Testing

Three layers, each with a clear job. Don't mix them — when a unit test reaches
for the database, it stops being a unit test and becomes a slow, fragile
integration test that nobody runs locally.

## The three layers

| Layer | What it proves | Lives in | I/O allowed | Run command |
|---|---|---|---|---|
| **Unit** | A single function or module behaves. | `src/` inside `#[cfg(test)] mod tests` | **none** | `cargo test --workspace --lib --bins` |
| **Integration** | Components wired together work — real DB, real HTTP fakes. | `crates/*/tests/**` | DB, `wiremock`, filesystem | `cargo test --workspace --tests --exclude e2e` |
| **E2E** | A deployed service answers correctly over real HTTP. | `crates/e2e/tests/**` | real endpoints | `cargo test -p e2e` |

The `crates/e2e` crate is optional — both repos add it once they have a
deployed staging environment. Don't start there.

> **Optional upgrade: `cargo nextest`.** `nextest` is a drop-in replacement
> that runs tests in parallel with better isolation and reporting (substitute
> `cargo nextest run …` for `cargo test …` above). Worth adopting once the
> suite is big enough that test runtime hurts. Neither repo enforces it
> today, so the commands above stay with `cargo test`.

## Tools

Use the same tools as the existing repos. Consistency makes test code
readable across crates.

- **`rstest`** — table-driven parametric tests. The cure for copy-pasted
  `#[test]` functions that differ in one input.
- **`pretty_assertions`** — drop-in replacement for `assert_eq!` with colored
  diffs. Drop the import in every test module.
- **`insta`** — snapshot testing for response shapes and large outputs. Commit
  the `.snap` files; review changes in PRs (`cargo insta review`).
- **`wiremock`** — every external HTTP provider gets a `wiremock` fake. Test
  success, error, timeout, and malformed-response cases.
- **`mockall`** — `#[automock]` on a trait gives you a generated mock with
  expectation builders. Useful for unit-testing service-layer code without
  pulling in a real provider impl.

## Unit tests

Live inline with the code. Compile only with `#[cfg(test)]`, so they don't
appear in the release binary.

```rust
// crates/domain/src/scope.rs
pub fn parse_list(input: &str) -> Vec<Scope> { /* ... */ }

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    #[rstest]
    #[case("profile:read", vec![Scope::ProfileRead])]
    #[case("profile:read profile:write", vec![Scope::ProfileRead, Scope::ProfileWrite])]
    #[case("", vec![])]
    #[case("  profile:read  ", vec![Scope::ProfileRead])]
    fn parses_scope_lists(#[case] input: &str, #[case] expected: Vec<Scope>) {
        assert_eq!(parse_list(input), expected);
    }
}
```

Hard rule: **no `tokio::main`, no `sqlx`, no `reqwest`, no `wiremock` in unit
tests.** If you need any of them, your test moved up a layer.

## Integration tests

Live in `crates/<crate>/tests/`. Each file in `tests/` is its own crate, so:

- They import only the public surface of `<crate>` and its workspace deps.
- They can use real Postgres (via a containerized DB), real `wiremock`, real
  axum routers spun up locally.
- They run inside `cargo test` / `cargo nextest` but are slower — both repos
  run them in their own CI job.

### Integration test for a handler

```rust
// crates/api/tests/me_endpoint.rs
use axum::http::{Method, Request, StatusCode};
use tower::ServiceExt;  // for `oneshot`
use library_api::{ApiState, router};
use serde_json::Value;

mod common;
use common::test_state_with_seeded_user;

#[tokio::test]
async fn integration_me_returns_envelope_with_user_summary() {
    let (state, user_id, access_token) = test_state_with_seeded_user().await;
    let router = router(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/me")
        .header("authorization", format!("Bearer {access_token}"))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body: Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap()
    ).unwrap();

    insta::assert_json_snapshot!(body, {
        ".data.created_at" => "[ts]",
    });
}
```

The `insta` redaction (`".data.created_at" => "[ts]"`) keeps the snapshot
stable across runs. Anything non-deterministic — timestamps, generated IDs,
random nonces — gets redacted.

### Integration tests against Postgres

Both repos take a pragmatic approach: spin Postgres up *once* per CI run, give
each test its own database (or schema) by template-cloning a baseline. The
helper module looks roughly like:

```rust
// crates/api/tests/common/mod.rs
use sqlx::PgPool;

pub async fn fresh_pool() -> PgPool {
    // Connect to the test Postgres instance, CREATE DATABASE with a unique
    // name, run migrations, return the pool. Drop the DB on teardown.
    // Both repos use a shared template DB to skip re-running migrations.
    unimplemented!()
}
```

In CI, Postgres runs as a `docker run -d postgres:17` step (see the CI skill's
`api.yml` template). Locally, `make api-test` boots the same container via
`docker-compose`.

### Integration tests for a provider with `wiremock`

```rust
// crates/providers/tests/privy_client.rs
use library_providers::PrivyClient;
use library_domain::IdentityProvider;
use wiremock::{Mock, MockServer, ResponseTemplate};
use wiremock::matchers::{method, path};

#[tokio::test]
async fn integration_privy_verify_returns_identity_on_200() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/sessions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "user_id": "did:privy:abc",
            "email": "u@example.test",
        })))
        .mount(&server).await;

    let client = PrivyClient::with_base_url(&server.uri(), "app_id", "secret");
    let id = client.verify_token("fake-jwt").await.unwrap();

    assert_eq!(id.external_id.0, "did:privy:abc");
}

#[tokio::test]
async fn integration_privy_verify_maps_401_to_invalid_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(401)).mount(&server).await;

    let client = PrivyClient::with_base_url(&server.uri(), "app_id", "secret");
    let err = client.verify_token("fake-jwt").await.unwrap_err();
    assert!(matches!(err, library_domain::IdentityError::InvalidToken));
}
```

The matrix per provider: **success, upstream-error, malformed-body, network-error
/ timeout.** A provider that only has a happy-path test will eventually betray
you in production.

## Naming integration tests

Prefix integration test function names with `integration_`:

```rust
#[tokio::test]
async fn integration_me_endpoint_returns_envelope() { /* ... */ }
```

This lets nextest filter at the granularity the existing repos use (`--filter
'/^integration_/'` or similar). It also makes test output skim-able.

## E2E tests

Optional — start when staging exists.

`crates/e2e` is **decoupled** from every internal crate. It imports nothing
from `library-*`; it talks to the deployed service over real HTTP, like a
third-party consumer would. The tests assert wire-level behavior, including
the `ApiResponse` envelope.

```toml
# crates/e2e/Cargo.toml
[package]
name = "e2e"
publish = false
[dependencies]
reqwest = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true }
```

```rust
// crates/e2e/tests/health.rs
#[tokio::test]
async fn e2e_health_returns_200() {
    let base = std::env::var("E2E_BASE_URL").expect("set E2E_BASE_URL to point at staging");
    let resp = reqwest::get(format!("{base}/livez")).await.unwrap();
    assert!(resp.status().is_success());
}
```

E2E reads its own config from env (`E2E_BASE_URL`, test credentials, etc.) —
this is the *one* place `env::var` is fine, because e2e isn't part of the
service binary.

## Test commands the repos already expose

```bash
make api-fmt-check    # rustfmt --check
make api-lint         # cargo clippy --workspace --all-targets -- -D warnings
make api-test         # cargo test (workspace)
make contract-check   # regenerate openapi.json and diff against committed
cargo insta review    # walk through pending snapshots
```

CI runs the same commands (see the sibling CI skill). Anything you can run
locally, you can verify before pushing.

## Common testing mistakes

- **Reaching for the database in a unit test.** Move the test to `tests/` and
  promote it to integration.
- **One giant `setup_everything()` per file.** Test setup that touches many
  tables and creates many users is a smell. Each test should care about
  the minimum state it asserts on.
- **`#[ignore]`'d tests.** Either the test is needed (fix it) or it isn't
  (delete it). `#[ignore]` is technical debt that compounds.
- **Forgetting `insta` redactions.** A snapshot that contains
  `"2024-01-15T13:42:11Z"` will fail on the next run. Always redact non-deterministic fields.
- **Hand-rolled HTTP fakes** instead of `wiremock`. The hand-rolled fakes drift
  from the real API; `wiremock` makes it cheap to add a 401-case alongside the
  200-case.
- **Mocking the database in integration tests.** Use a real ephemeral Postgres.
  Mocking SQL gives you green tests against a broken schema.
- **Tests that depend on each other's order.** Each `#[test]` / `#[tokio::test]`
  must be independent. Nextest runs them in parallel; ordering bugs surface
  immediately as flakes.
- **Snapshotting the whole response without redacting `expires_at` and
  `created_at`.** Brittle. Redact, or split into a "structural" snapshot and
  separate exact-value assertions for the few fields you care about.
