# Config and providers

These two concerns travel together because they're the seam between "the world"
and the application core. Config is how runtime knobs reach the code; providers
are how external services reach the code. Both arrive through *typed structs
and trait objects*, never through ambient state.

## Config: figment over `std::env::var`

`std::env::var` is on the `clippy.toml` disallowed list for a reason — it
scatters string-typed reads across the codebase, makes startup failures look
like first-request failures, and is unsound under multithreading. Replace it
with one typed config struct loaded once at boot.

### `*-config/src/lib.rs` template

```rust
use figment::{Figment, providers::{Env, Format, Toml, Serialized}};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LibraryConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub auth: AuthConfig,
    pub privy: PrivyConfig,
    pub stripe: StripeConfig,
    pub rate_limit: RateLimitConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AuthConfig {
    pub jwt: JwtConfig,
    pub session_ttl_seconds: u32,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct JwtConfig {
    pub issuer: String,
    pub audience: String,
    pub private_key_pem: String,
    pub key_id: String,
    pub access_ttl_seconds: u32,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PrivyConfig {
    pub app_id: String,
    pub app_secret: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct StripeConfig {
    pub secret_key: String,
    pub webhook_secret: String,
    pub publishable_key: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct RateLimitConfig {
    pub per_ip_per_minute: u32,
}

impl Default for LibraryConfig {
    fn default() -> Self {
        // Dev-only defaults. Production values come from env (figment merges them on top).
        // Leave secret-ish fields as obviously-fake strings so a misconfigured prod boot fails loudly.
        Self {
            server: ServerConfig { host: "127.0.0.1".into(), port: 8080 },
            database: DatabaseConfig {
                url: "postgres://localhost/library".into(),
                max_connections: 8,
            },
            auth: AuthConfig {
                jwt: JwtConfig {
                    issuer: "library-dev".into(),
                    audience: "library-dev".into(),
                    private_key_pem: String::new(),
                    key_id: "dev".into(),
                    access_ttl_seconds: 900,
                },
                session_ttl_seconds: 60 * 60 * 24 * 7,
            },
            privy: PrivyConfig { app_id: "dev".into(), app_secret: "dev".into() },
            stripe: StripeConfig {
                secret_key: "dev".into(),
                webhook_secret: "dev".into(),
                publishable_key: "dev".into(),
            },
            rate_limit: RateLimitConfig { per_ip_per_minute: 60 },
        }
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error(transparent)]
    Figment(#[from] figment::Error),
}

pub fn load() -> Result<LibraryConfig, ConfigError> {
    Ok(Figment::new()
        .merge(Serialized::defaults(LibraryConfig::default()))
        .merge(Toml::file("config.toml"))
        .merge(Env::prefixed("LIBRARY_").split("__"))
        .extract()?)
}
```

Merge order:

1. **Defaults** — hardcoded `LibraryConfig::default()`. Enough to boot in dev.
2. **`config.toml`** in the working directory if present. Optional file, ignored
   if absent.
3. **Env vars** with the `LIBRARY_` prefix. Double-underscore separates levels:
   `LIBRARY_SERVER__PORT=8443`, `LIBRARY_DATABASE__MAX_CONNECTIONS=20`.

Env wins because container platforms inject env. The `config.toml` is for
committed defaults you'd version with the code. Defaults are the fallback so
a fresh checkout boots.

### How a section reaches its consumer

The binary owns the full config. It hands sections — not the whole struct — to
the components that need them:

```rust
// app/main.rs
let database = library_persistence::connect(&config.database).await?;
let privy = library_providers::PrivyClient::from_config(&config.privy);
let issuer = library_services::TokenIssuer::try_new(&config.auth.jwt)?;
```

Constructors in library crates always take `&FooConfig`, never the root struct
and never raw strings. This keeps each crate self-describing — `PrivyClient::from_config`
declares exactly what config it needs in its signature.

### Testing config without env-pollution

Don't mutate `std::env` inside tests (it's UB-prone). Build a config struct
in-place:

```rust
fn test_config() -> LibraryConfig {
    LibraryConfig {
        privy: PrivyConfig { app_id: "test".into(), app_secret: "test".into() },
        // ...rest from default
        ..LibraryConfig::default()
    }
}
```

## Providers: traits in domain, impls in providers

A "provider" is anything the service talks to over a wire it doesn't own —
Privy, Stripe, GitHub, an SMTP server, an LLM API. Each one gets a **trait in
`*-domain`** and an **impl in `*-providers`**. The rest of the code only sees
the trait.

### The trait lives in domain

```rust
// crates/domain/src/lib.rs
use async_trait::async_trait;
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Deserialize)]
pub struct VerifiedIdentity {
    pub external_id: UserExternalId,
    pub email: Option<String>,
}

pub struct UserExternalId(pub String);

#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("invalid token")]
    InvalidToken,
    #[error("provider unavailable: {reason}")]
    Upstream { reason: String },
}

#[async_trait]
pub trait IdentityProvider: Send + Sync {
    async fn verify_token(&self, token: &str) -> Result<VerifiedIdentity, IdentityError>;
}
```

Why traits in `domain` and not `providers`: the trait *is* the contract the
domain logic relies on. `services` writes its business logic against
`IdentityProvider`, not `PrivyClient`. That means `services` doesn't depend on
`providers`, doesn't pull `reqwest` into its tests, and can be unit-tested with
a hand-written mock or a `mockall::automock`-generated one.

### The impl lives in providers

```rust
// crates/providers/src/lib.rs
use async_trait::async_trait;
use library_config::PrivyConfig;
use library_domain::{IdentityProvider, IdentityError, UserExternalId, VerifiedIdentity};

pub struct PrivyClient {
    http: reqwest::Client,
    app_id: String,
    app_secret: String,
}

impl PrivyClient {
    pub fn from_config(config: &PrivyConfig) -> Self {
        Self {
            http: reqwest::Client::new(),
            app_id: config.app_id.clone(),
            app_secret: config.app_secret.clone(),
        }
    }
}

#[async_trait]
impl IdentityProvider for PrivyClient {
    async fn verify_token(&self, token: &str) -> Result<VerifiedIdentity, IdentityError> {
        let resp = self.http.post("https://auth.privy.io/api/v1/sessions")
            .basic_auth(&self.app_id, Some(&self.app_secret))
            .header("privy-app-id", &self.app_id)
            .json(&serde_json::json!({ "token": token }))
            .send().await
            .map_err(|e| IdentityError::Upstream { reason: e.to_string() })?;

        if !resp.status().is_success() {
            return Err(IdentityError::InvalidToken);
        }

        let body: PrivySession = resp.json().await
            .map_err(|e| IdentityError::Upstream { reason: e.to_string() })?;

        Ok(VerifiedIdentity {
            external_id: UserExternalId(body.user_id),
            email: body.email,
        })
    }
}

#[derive(Debug, serde::Deserialize)]
struct PrivySession { user_id: String, email: Option<String> }
```

### Wiring into `ApiState`

`ApiState` holds providers as `Arc<dyn Trait>`, not concrete types. This lets
tests swap in fakes without exposing concrete provider modules to test code:

```rust
// crates/api/src/lib.rs
use std::sync::Arc;
use library_domain::{IdentityProvider, PaymentGateway};

#[derive(Clone)]
pub struct ApiState {
    pub config: LibraryConfig,
    pub database: sqlx::PgPool,
    pub identity_provider: Arc<dyn IdentityProvider>,
    pub payment_gateway: Arc<dyn PaymentGateway>,
}

impl ApiState {
    pub fn new(
        config: LibraryConfig,
        database: sqlx::PgPool,
        identity_provider: Arc<dyn IdentityProvider>,
        payment_gateway: Arc<dyn PaymentGateway>,
    ) -> Self {
        Self { config, database, identity_provider, payment_gateway }
    }
}
```

And in `app/main.rs`:

```rust
let identity_provider: Arc<dyn IdentityProvider> = Arc::new(PrivyClient::from_config(&config.privy));
let payment_gateway: Arc<dyn PaymentGateway> = Arc::new(StripeClient::from_config(&config.stripe));
let state = ApiState::new(config, database, identity_provider, payment_gateway);
```

The `as Arc<dyn Trait>` coercion happens automatically because `Arc::new`
returns the concrete type, then the binding's type annotation widens it.

### Testing providers with `wiremock`

Provider crates test against `wiremock`, not against the real upstream. This is
where the success / error / timeout matrix lives — see [testing.md](testing.md).

### A second provider in the same crate

Don't make one `*-providers` per upstream. Both Privy and Stripe live together
in `*-providers/src/lib.rs` (or split into submodules within the crate). They
share `reqwest`, error patterns, and configuration plumbing. Reach for a
separate crate only if the dep set diverges sharply — e.g. an LLM provider
that pulls in a heavy SDK.

## When config bleeds into your code

If you find a string like `"https://auth.privy.io"` hardcoded, that's a
config-shaped value. Move it. The rule of thumb: **if a value would differ
between dev / staging / production, it's config.** Even base URLs go in the
provider's config section (it lets you point staging at a local mock).

```rust
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PrivyConfig {
    pub app_id: String,
    pub app_secret: String,
    #[serde(default = "default_privy_base_url")]
    pub base_url: String,  // overridable in tests / staging
}

fn default_privy_base_url() -> String { "https://auth.privy.io".into() }
```

## Common config & provider mistakes

- **Putting secrets in `Default::default()`.** Defaults exist for the dev path;
  prod values come from env. If a default *is* a real-looking secret, you'll
  ship it. Make defaults obviously fake (`"test"`, `"dev-only"`) or omit them
  from the secret fields and panic at boot if missing.
- **Reading config in the handler.** `state.config.privy.app_id` inside a
  handler is a code smell — that's the provider's job. Pass the section to the
  provider at construction.
- **Forgetting `Send + Sync` on the trait.** Without `Send + Sync`, you can't
  store `Arc<dyn Trait>` in shared state. The trait declaration must include
  it explicitly: `pub trait IdentityProvider: Send + Sync { ... }`.
- **Concrete types in `ApiState`.** `ApiState { privy: PrivyClient, ... }` means
  tests need to construct a real `PrivyClient`. Always `Arc<dyn IdentityProvider>`.
- **Letting a provider impl `From<PrivyHttpError> for ApiError`.** That couples
  the API layer to provider internals. The trait returns a domain error
  (`IdentityError`); the API layer maps `IdentityError` to `ApiError`.
- **Hot-reloading config.** Don't. Reload by restarting the process. Live
  config-mutation is a complexity tax with little payoff at this scale.
