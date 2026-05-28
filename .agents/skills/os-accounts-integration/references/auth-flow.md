# Login with Open Software — auth flow

Login is a **redirect + PKCE + one-time-code exchange**. Your app never embeds a
login SDK and never sees an upstream identity-provider token; the hosted OS Accounts login page
performs the sign-in and hands your app a short-lived **authorization code**, which
your *backend* exchanges for an OS Accounts **JWT pair**. PKCE binds the code to the
browser that started the flow; `state` defends against CSRF.

Registered consumer apps should use an OAuth `client_id` (`ocl_...`) created in
the OS Accounts Admin console. The hosted portal owns the Privy/GitHub login and
any consent review; your app only redirects there and receives the final code.

## Sequence

```
1. Browser hits your app's  GET /auth/start
   → you generate a PKCE verifier + state, store both in httpOnly cookies,
     and redirect to:
     {ACCOUNTS}/login?client_id={OS_ACCOUNTS_CLIENT_ID}
                   &redirect_uri={APP_ORIGIN}/auth/callback
                   &scope=profile:read billing:read
                   &state=<state>
                   &code_challenge=<base64url(sha256(verifier))>
                   &code_challenge_method=S256
2. User signs in at OS Accounts and, if needed, reviews hosted consent (not your concern).
3. OS Accounts redirects back → {APP_ORIGIN}/auth/callback?code=<one-time>&state=<state>
4. Your GET /auth/callback verifies state, then your BACKEND calls
   POST {API}/auth/token { grant_type:"authorization_code", code, code_verifier, redirect_uri }
   → { access_token, refresh_token }
5. You store both tokens in httpOnly cookies — the user is now signed in to YOUR app.
```

`client_id` must identify an active OAuth client, and `redirect_uri` must exactly
match that client's allowed redirect URI (set in the OS Accounts Admin console)
or OS Accounts rejects the request. The redirect URI must also stay within the
parent App's registered origins.

## 1. Start route — redirect with PKCE

`app/auth/start/route.ts`:

```ts
import { createHash, randomBytes } from "node:crypto";
import { cookies } from "next/headers";
import { NextResponse } from "next/server";
import { getServerEnv } from "@/lib/env/server";

export async function GET() {
  const verifier = base64Url(randomBytes(32)); // PKCE code_verifier
  const state = base64Url(randomBytes(24)); // CSRF token
  const env = getServerEnv();
  const cookieStore = await cookies();
  const secure = new URL(env.APP_ORIGIN).protocol === "https:";

  // httpOnly so the browser carries them back but JS cannot read them.
  cookieStore.set("oa_pkce", verifier, { httpOnly: true, sameSite: "lax", secure, path: "/" });
  cookieStore.set("oa_state", state, { httpOnly: true, sameSite: "lax", secure, path: "/" });

  const login = new URL("/login", env.OS_ACCOUNTS_URL);
  login.searchParams.set("client_id", env.OS_ACCOUNTS_CLIENT_ID);
  login.searchParams.set("redirect_uri", `${env.APP_ORIGIN}/auth/callback`);
  login.searchParams.set("scope", "profile:read billing:read");
  login.searchParams.set("state", state);
  login.searchParams.set("code_challenge", base64Url(createHash("sha256").update(verifier).digest()));
  login.searchParams.set("code_challenge_method", "S256");
  return NextResponse.redirect(login);
}

function base64Url(buf: Buffer) {
  return buf.toString("base64").replaceAll("+", "-").replaceAll("/", "_").replaceAll("=", "");
}
```

Your "Login with Open Software" button is just a link/redirect to `/auth/start`.
Only ask for scopes the feature needs. Request `credits:spend` only when your
confidential backend also meters with the App API key.

## 2. Callback route — exchange the code

`app/auth/callback/route.ts`:

```ts
import { cookies } from "next/headers";
import { type NextRequest, NextResponse } from "next/server";
import { requestApiJson } from "@/lib/api/server";
import { setTokenCookies } from "@/lib/auth/session";
import { getServerEnv } from "@/lib/env/server";

type TokenPair = { access_token: string; refresh_token: string };

export async function GET(request: NextRequest) {
  const env = getServerEnv();
  const code = request.nextUrl.searchParams.get("code");
  const state = request.nextUrl.searchParams.get("state");
  const cookieStore = await cookies();
  const verifier = cookieStore.get("oa_pkce")?.value;
  const expectedState = cookieStore.get("oa_state")?.value;

  // Reject if the CSRF state doesn't match or the PKCE verifier is missing.
  if (!code || !verifier || state !== expectedState) {
    return NextResponse.redirect(new URL("/?auth=failed", env.APP_ORIGIN));
  }

  const token = await requestApiJson<TokenPair>("/auth/token", {
    method: "POST",
    body: {
      grant_type: "authorization_code",
      code,
      code_verifier: verifier,
      redirect_uri: `${env.APP_ORIGIN}/auth/callback`,
    },
  });
  if (!token.success || !token.data) {
    return NextResponse.redirect(new URL("/?auth=failed", env.APP_ORIGIN));
  }

  await setTokenCookies(token.data); // httpOnly access + refresh
  return NextResponse.redirect(new URL("/", env.APP_ORIGIN));
}
```

## 3. Session helpers — store + refresh + logout

`lib/auth/session.ts`:

```ts
import "server-only";
import { cookies } from "next/headers";
import { requestApiJson } from "@/lib/api/server";
import { getServerEnv } from "@/lib/env/server";

type TokenPair = { access_token: string; refresh_token: string };
type RefreshResult = { ok: true; accessToken: string } | { ok: false; message: string };

export async function setTokenCookies(token: TokenPair) {
  const env = getServerEnv();
  const cookieStore = await cookies();
  const secure = new URL(env.APP_ORIGIN).protocol === "https:";
  const opts = { httpOnly: true, sameSite: "lax" as const, secure, path: "/" };
  cookieStore.set("oa_access", token.access_token, opts);
  cookieStore.set("oa_refresh", token.refresh_token, opts);
}

// Refresh tokens ROTATE: each refresh invalidates the presented token and
// returns a new pair. Reusing an old refresh token revokes the whole family
// server-side — so always persist the new pair and never replay an old one.
export async function refreshSession(requestId: string): Promise<RefreshResult> {
  const cookieStore = await cookies();
  const refreshToken = cookieStore.get("oa_refresh")?.value;
  if (!refreshToken) return { ok: false, message: "Sign in first" };

  try {
    const token = await requestApiJson<TokenPair>("/auth/refresh", {
      method: "POST",
      requestId,
      body: { refresh_token: refreshToken },
    });
    if (!token.success || !token.data) {
      await clearTokenCookies();
      return { ok: false, message: token.message ?? "Sign in first" };
    }
    await setTokenCookies(token.data);
    return { ok: true, accessToken: token.data.access_token };
  } catch {
    await clearTokenCookies();
    return { ok: false, message: "Sign in first" };
  }
}

export async function clearTokenCookies() {
  const cookieStore = await cookies();
  cookieStore.delete("oa_access");
  cookieStore.delete("oa_refresh");
}
```

Logout (`app/auth/logout/route.ts`): read the `oa_refresh` cookie, `POST
{API}/auth/logout { refresh_token }`, then `clearTokenCookies()` and redirect home.

## Notes

- Access tokens are short-lived (~15 min) ES256 JWTs; refresh tokens last ~30 days. You don't validate the JWT yourself — the API does. If you ever need to (e.g. an edge gateway), verify against `GET {API}/.well-known/jwks.json`: require `alg=ES256`, a known `kid`, and exact `iss`/`aud`.
- Refresh tokens are associated with the OAuth client when login used
  `client_id`. If the client scopes are narrowed later, the next refresh returns
  tokens whose scopes are intersected with the live client policy.
- `sameSite: "lax"` is enough because the callback is a top-level redirect (GET). Use `secure: true` in production (https origins).
- The cookie names (`oa_*`) are yours — pick anything stable and app-specific.
