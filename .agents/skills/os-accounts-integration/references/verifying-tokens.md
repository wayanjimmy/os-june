# Verifying OS Accounts tokens in a backend (resource server)

Use this when a **backend service authenticates requests itself** — no Next.js,
no BFF, no browser. The classic case: an **AI proxy / API service** that clients
(or another app's BFF) call with the user's OS Accounts **access token** in
`Authorization: Bearer <jwt>`. Your service must decide *who the user is* on every
request, locally and fast, without calling OS Accounts each time.

This is **authentication** (who is this user → the `usr_` id). Deciding whether
they may spend (balance / metering) is separate — see
[metering-and-billing.md](metering-and-billing.md). A typical AI proxy does both:
verify the token to get the `usr_` id, then `authorize`→`charge` with its App API
key.

## What "validate the token" means

The access token is an **ES256 JWT** signed by OS Accounts' private key. You verify
it against the **public** keys published at the JWKS endpoint. A correct verifier
**must**:

- **Require `alg: ES256`.** Reject `none` and every other algorithm — never trust the token's own `alg` to pick the verification method.
- **Match the `kid`** in the token header to a key in the JWKS (supports key rotation: active + retiring keys are both published).
- **Check `iss` exactly** == the issuer (`https://accounts.opensoftware.co`).
- **Check `aud` exactly** == your expected audience (`open-software-apps`).
- **Check `exp`** (and `nbf`/`iat`) with a **small clock leeway** (~5s). Access tokens are short-lived (~15 min) — an expired token is a `401`, not a server error.
- **Cache the JWKS** with a TTL and refetch on an unknown `kid` (so rotation doesn't require a redeploy). Never fetch JWKS per request.

On success you get the claims; `sub` is the user's external `usr_` id — the only
user identifier you persist or pass on the charge path. **You never refresh** the
token; refreshing is the session holder's job. If it's expired, return `401` and
let the caller refresh upstream.

> **Gotcha — issuer ≠ JWKS host.** The token's `iss` is the accounts site
> (`https://accounts.opensoftware.co`), but JWKS is served by the **API**:
> `https://api.accounts.opensoftware.co/.well-known/jwks.json`. Configure them
> independently; don't derive one from the other.

## Config (server-only)

| Var | Purpose | Example |
|---|---|---|
| `OS_ACCOUNTS_ISSUER` | Expected `iss` claim | `https://accounts.opensoftware.co` |
| `OS_ACCOUNTS_AUDIENCE` | Expected `aud` claim | `open-software-apps` |
| `OS_ACCOUNTS_JWKS_URL` | JWKS endpoint (on the API origin) | `https://api.accounts.opensoftware.co/.well-known/jwks.json` |

---

## TypeScript — shared verifier (`jose`)

`jose`'s `createRemoteJWKSet` does the heavy lifting: it caches the JWKS, selects
the key by `kid`, refetches on rotation (with a cooldown), and — because you pass
`algorithms: ["ES256"]` — rejects `none`/alg-confusion attacks.

`lib/os-accounts/verify.ts`:

```ts
import { createRemoteJWKSet, jwtVerify, type JWTPayload } from "jose";

// Module-scoped: the JWKS cache must be reused across requests, not rebuilt.
const jwks = createRemoteJWKSet(new URL(process.env.OS_ACCOUNTS_JWKS_URL!), {
  cooldownDuration: 30_000, // min ms between forced refetches on unknown kid
  cacheMaxAge: 600_000, // refetch keys at most every 10 min
});

export type OsUser = { id: string; claims: JWTPayload };

/** Verifies an OS Accounts access token. Throws if invalid/expired. */
export async function verifyAccessToken(token: string): Promise<OsUser> {
  const { payload } = await jwtVerify(token, jwks, {
    issuer: process.env.OS_ACCOUNTS_ISSUER,
    audience: process.env.OS_ACCOUNTS_AUDIENCE,
    algorithms: ["ES256"], // reject none + alg confusion
    clockTolerance: 5, // seconds of leeway
  });
  if (!payload.sub?.startsWith("usr_")) throw new Error("unexpected subject");
  return { id: payload.sub, claims: payload };
}

export function bearer(header: string | null | undefined): string | null {
  return header?.startsWith("Bearer ") ? header.slice(7).trim() : null;
}
```

Return the ecosystem envelope on failure so an upstream caller that knows the
convention can react (`error_code 3001` = expired → refresh + retry):

```ts
export const UNAUTHENTICATED = {
  success: false,
  data: null,
  error_code: 3001,
  message: "Invalid or expired token",
} as const;
```

### Hono

```ts
import { createMiddleware } from "hono/factory";
import { bearer, verifyAccessToken, UNAUTHENTICATED } from "../lib/os-accounts/verify";

export const requireUser = createMiddleware<{ Variables: { userId: string } }>(
  async (c, next) => {
    const token = bearer(c.req.header("authorization"));
    if (!token) return c.json(UNAUTHENTICATED, 401);
    try {
      const user = await verifyAccessToken(token);
      c.set("userId", user.id); // available downstream via c.get("userId")
      await next();
    } catch {
      return c.json(UNAUTHENTICATED, 401);
    }
  },
);

// app.use("/v1/*", requireUser)  →  c.get("userId") is the usr_ id
```

### Fastify

```ts
import fp from "fastify-plugin";
import { bearer, verifyAccessToken, UNAUTHENTICATED } from "../lib/os-accounts/verify";

declare module "fastify" {
  interface FastifyRequest { userId?: string }
}

export default fp(async (fastify) => {
  fastify.decorateRequest("userId", undefined);
  fastify.addHook("preHandler", async (req, reply) => {
    const token = bearer(req.headers.authorization);
    if (!token) return reply.code(401).send(UNAUTHENTICATED);
    try {
      req.userId = (await verifyAccessToken(token)).id;
    } catch {
      return reply.code(401).send(UNAUTHENTICATED);
    }
  });
});
```

### Express

```ts
import type { RequestHandler } from "express";
import { bearer, verifyAccessToken, UNAUTHENTICATED } from "../lib/os-accounts/verify";

export const requireUser: RequestHandler = async (req, res, next) => {
  const token = bearer(req.headers.authorization);
  if (!token) return res.status(401).json(UNAUTHENTICATED);
  try {
    res.locals.userId = (await verifyAccessToken(token)).id;
    next();
  } catch {
    res.status(401).json(UNAUTHENTICATED);
  }
};
// app.use("/v1", requireUser)  →  res.locals.userId
```

---

## Rust — axum extractor (`jsonwebtoken`)

Cache the JWKS as `kid → DecodingKey` behind an `RwLock` with a TTL; refresh on a
miss. An axum `FromRequestParts` extractor turns a valid `Bearer` token into the
`usr_` id, so handlers just take `OsUser` as an argument.

`Cargo.toml`:

```toml
[dependencies]
axum = "0.7"
jsonwebtoken = "9"
reqwest = { version = "0.12", features = ["json"] }
serde = { version = "1", features = ["derive"] }
tokio = { version = "1", features = ["sync"] }
```

```rust
use std::{collections::HashMap, sync::Arc, time::{Duration, Instant}};

use axum::{extract::FromRequestParts, http::{request::Parts, StatusCode}};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct Verifier {
    issuer: String,
    audience: String,
    jwks_url: String,
    http: reqwest::Client,
    keys: Arc<RwLock<KeyCache>>,
}

#[derive(Default)]
struct KeyCache { by_kid: HashMap<String, DecodingKey>, fetched_at: Option<Instant> }

const JWKS_TTL: Duration = Duration::from_secs(600);

#[derive(Deserialize)]
struct Jwks { keys: Vec<Jwk> }
#[derive(Deserialize)]
struct Jwk { kid: String, x: String, y: String }

#[derive(Debug, Deserialize)]
pub struct AccessClaims { pub sub: String, pub exp: usize }

impl Verifier {
    pub fn new(issuer: String, audience: String, jwks_url: String) -> Self {
        Self { issuer, audience, jwks_url, http: reqwest::Client::new(),
               keys: Arc::new(RwLock::new(KeyCache::default())) }
    }

    async fn key_for(&self, kid: &str) -> Option<DecodingKey> {
        let fresh = {
            let c = self.keys.read().await;
            matches!(c.fetched_at, Some(t) if t.elapsed() < JWKS_TTL) && c.by_kid.contains_key(kid)
        };
        if !fresh { self.refresh().await; }      // refetch on miss or stale
        self.keys.read().await.by_kid.get(kid).cloned()
    }

    async fn refresh(&self) {
        let Ok(resp) = self.http.get(&self.jwks_url).send().await else { return };
        let Ok(jwks) = resp.json::<Jwks>().await else { return };
        let mut map = HashMap::new();
        for k in jwks.keys {
            if let Ok(dk) = DecodingKey::from_ec_components(&k.x, &k.y) {
                map.insert(k.kid, dk);
            }
        }
        let mut c = self.keys.write().await;
        c.by_kid = map;
        c.fetched_at = Some(Instant::now());
    }

    pub async fn verify(&self, token: &str) -> Result<AccessClaims, ()> {
        let header = decode_header(token).map_err(|_| ())?;
        if header.alg != Algorithm::ES256 { return Err(()); }   // reject none/others
        let kid = header.kid.ok_or(())?;
        let key = self.key_for(&kid).await.ok_or(())?;          // unknown kid → unauthenticated
        let mut v = Validation::new(Algorithm::ES256);
        v.set_issuer(&[self.issuer.as_str()]);
        v.set_audience(&[self.audience.as_str()]);
        v.leeway = 5;
        decode::<AccessClaims>(token, &key, &v).map(|d| d.claims).map_err(|_| ())
    }
}

/// Handlers take `OsUser` to require a valid token; `.0` is the usr_ id.
pub struct OsUser(pub String);

impl<S> FromRequestParts<S> for OsUser
where
    Verifier: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let verifier = Verifier::from_ref(state);
        let token = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "))
            .ok_or((StatusCode::UNAUTHORIZED, "missing token"))?;
        let claims = verifier.verify(token).await.map_err(|()| (StatusCode::UNAUTHORIZED, "invalid token"))?;
        Ok(OsUser(claims.sub))
    }
}
// use axum::extract::FromRef;  async fn handler(OsUser(user_id): OsUser) { ... }
```

The extractor returns `401` on any failure (missing/expired/unknown-kid/bad-alg).
Wrap the body in your API's `ApiResponse` envelope if you want `error_code: 3001`
on the wire for ecosystem consistency.

## Verify it works

- A valid token → handler sees the `usr_` id; an expired one → `401`.
- A token signed by a different key, `alg: none`, wrong `iss`/`aud`, or an unknown `kid` → `401`.
- Rotate the signing key on OS Accounts: in-flight tokens keep verifying (retiring key still in JWKS) and new tokens verify after the cache TTL — no redeploy.
- JWKS is fetched at most once per TTL, not per request (check your logs).
