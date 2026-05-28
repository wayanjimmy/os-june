# Testing and troubleshooting OS Accounts integrations

Use this when an integration fails in staging/local or when the user asks "why
does login not come back?", "Origin not allowed", "redirect_uri is not
registered", "token exchange failed", or "can I demo this with Tauri?" Diagnose
the failing hop before changing app code.

## Staging defaults

```sh
OS_ACCOUNTS_URL=https://os-accounts-portal-staging.up.railway.app
OS_ACCOUNTS_API_URL=https://os-accounts-api-staging.up.railway.app
```

For desktop demos using a custom URL scheme, start with:

```text
redirect_uri=yourapp://auth/callback
```

Register that exact redirect URI on the OAuth client in the Admin console. If
the app uses loopback instead, register one fixed loopback origin on the parent
App (for example `http://localhost:3417`) and ensure the OS Accounts API has
`ACCOUNTS_SERVER__ALLOW_LOOPBACK_APP_URLS=true`.

For current OAuth-client integrations, prefer the OS Accounts Admin console:
create the App, create an OAuth client, copy the `ocl_...` client id into the
consumer app, and keep redirect URIs/scopes on the OAuth client. Manual SQL is a
break-glass path, not the default onboarding path.

## Expected desktop network path

```text
Tauri app opens system browser
  -> OS_ACCOUNTS_URL/login?client_id=ocl_...&redirect_uri=...&state=...&code_challenge=...
  -> portal starts Privy GitHub login
  -> Privy redirects back to the portal
  -> portal calls /bff/auth/login
  -> OS Accounts returns one-time code
  -> portal redirects to yourapp://auth/callback?code=...&state=...
  -> Tauri Rust core verifies state and POSTs /auth/token
  -> Rust core stores access/refresh tokens in the OS keychain
```

The code appears in the app callback. The access token must not appear in the
callback URL, browser localStorage, webview localStorage, logs, or docs.

## Privy checks

If the browser shows `Origin not allowed`, check Privy first, not Tauri code.
The Privy app must allow the portal origin that is running the login page:

```text
https://os-accounts-portal-staging.up.railway.app
http://localhost:3000
```

For production, add the production portal domain. Do not add Tauri custom schemes
to Privy origins; Privy talks to the hosted portal, not directly to the desktop
callback.

## OS Accounts app registration checks

Prefer the Admin console first. For database-level diagnosis, query both the App
and OAuth client:

```sql
SELECT
  a.external_id AS app_id,
  a.name AS app_name,
  a.status,
  a.allowed_origins AS app_allowed_origins,
  c.client_id,
  c.client_type,
  c.allowed_origins AS client_allowed_origins,
  c.allowed_redirect_uris,
  c.allowed_scopes
FROM apps a
JOIN oauth_clients c ON c.app_id = a.id
WHERE c.client_id = 'ocl_your_client';
```

Expected:

- `status = 'active'`
- Browser/web flow: the login URL includes `client_id=ocl_...`, and
  `allowed_redirect_uris` contains the exact callback URI
- Deep link flow: the OAuth client's `allowed_redirect_uris` contains
  `yourapp://auth/callback`
- Loopback flow: `allowed_origins` contains the fixed local origin, and the API
  flag `ACCOUNTS_SERVER__ALLOW_LOOPBACK_APP_URLS=true` is set
- Requested `scope` is a subset of the OAuth client's `allowed_scopes`
- No two active apps share the same loopback origin/port, or login attribution is
  ambiguous

## Manual token exchange check

After a successful browser login, copy the `code` from the desktop callback and
use the same `code_verifier` generated at login start:

```sh
curl -sS "$OS_ACCOUNTS_API_URL/auth/token" \
  -H 'content-type: application/json' \
  --data-binary '{
    "grant_type": "authorization_code",
    "code": "<code-from-callback>",
    "code_verifier": "<original-pkce-verifier>",
    "redirect_uri": "yourapp://auth/callback"
  }'
```

Expected response envelope:

```json
{
  "success": true,
  "data": {
    "access_token": "...",
    "refresh_token": "..."
  }
}
```

If this succeeds manually but the app fails, the issue is in Tauri state,
deep-link parsing, keychain storage, or refresh handling. If this fails manually,
fix OS Accounts app registration, PKCE state/verifier handling, or the API env.

## Common failures

| Symptom | Likely source | Check |
|---|---|---|
| `Origin not allowed` in browser | Privy configuration | Add the portal domain to Privy allowed origins/domains. |
| `redirect_uri is not registered` | OS Accounts OAuth client registration | Add the exact callback URI to the OAuth client's redirect URIs, and ensure it stays within the parent App origin. |
| `requested scope is not allowed for this client` | OAuth client scope policy | In Admin console, add the needed scope to the OAuth client or request fewer scopes. |
| Consent screen appears before callback | Expected OAuth behavior | The hosted portal is asking the User to approve scopes for this `client_id`; your app should just wait for the final redirect. |
| Browser completes login but app never wakes | Deep link registration | On macOS, the custom scheme must be in `tauri.conf.json` deep-link config and tested from a bundled app; runtime `register()` is not supported. On Windows/Linux dev, check `register_all()`/`register()`, single-instance, and argv handling. |
| App wakes but token exchange fails | PKCE/state mismatch | Use the original verifier; verify returned state before exchange; exchange only once. |
| `/me` returns `error_code: 3001` | Expired access token | Call `/auth/refresh`, persist the rotated pair, retry once. |
| Paid action works only from local scripts | App API key placement | The desktop app must call your backend broker; never ship `osk_` in Tauri. |
| Balance does not update immediately after top-up | Stripe webhook timing | Re-fetch balance after the user returns; do not grant credits locally. |

## Pre-commit leak check

Before committing integration work:

```sh
git diff --cached --name-only | xargs rg -n "osk_|access_token|refresh_token|PRIVY_APP_SECRET|GITHUB_TOKEN|password"
```

Investigate every match. Placeholders are fine; real secrets and copied test
account credentials are not.
