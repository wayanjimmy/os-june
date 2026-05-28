# BFF, envelope, env & error codes

The browser must never hold an OS Accounts token or your App API key. So all
calls from the browser to OS Accounts go through a **same-origin BFF** (Backend
For Frontend): the browser calls *your* `/bff/*`, your server reads the httpOnly
session cookie, attaches the `Bearer` access token, and forwards to the API.
Tokens and keys stay on the server; the browser only ever holds an httpOnly
cookie it can't read.

```
browser → GET /bff/billing/balance        (httpOnly cookie rides along)
  your BFF → reads oa_access cookie, sets Authorization: Bearer <jwt>
           → GET {API}/billing/balance     (server-to-server, node:http)
           ← envelope
  ← envelope (token never exposed)
```

## The forwarder — use `node:http`, not `fetch`

Under **Bun**, `fetch` has a request-body/streaming bug (#29515) that breaks
server-to-server forwarding. Use the Node `http`/`https` modules directly. This
client also centralises the envelope type, the `x-request-id` for tracing, and
JSON encoding.

`lib/api/server.ts`:

```ts
import { randomUUID } from "node:crypto";
import http from "node:http";
import https from "node:https";
import { getServerEnv } from "@/lib/env/server";

export type ApiEnvelope<T> = {
  data: T | null;
  success: boolean;
  error_code?: number;
  message?: string;
};

type ApiRequestOptions = {
  method?: string;
  body?: unknown;
  authorization?: string; // "Bearer <jwt>" or "Bearer osk_…"
  requestId?: string;
};

export async function requestApiJson<T>(
  path: string,
  options: ApiRequestOptions = {},
): Promise<ApiEnvelope<T>> {
  const body = options.body === undefined ? undefined : JSON.stringify(options.body);
  const headers: Record<string, string> = {
    accept: "application/json",
    "x-request-id": options.requestId ?? randomUUID(),
  };
  if (body !== undefined) {
    headers["content-type"] = "application/json";
    headers["content-length"] = Buffer.byteLength(body).toString();
  }
  if (options.authorization) headers.authorization = options.authorization;

  return requestJson<ApiEnvelope<T>>(apiUrl(path), {
    method: options.method ?? "GET",
    headers,
    body,
  });
}

export function apiUrl(path: string) {
  const origin = getServerEnv().OS_ACCOUNTS_API_URL.replace(/\/$/, "");
  return new URL(path.replace(/^\//, ""), `${origin}/`);
}

export function appApiKey() {
  return getServerEnv().OS_ACCOUNTS_APP_API_KEY;
}

function requestJson<T>(
  url: URL,
  options: { method: string; headers: Record<string, string>; body?: string },
): Promise<T> {
  return new Promise((resolve, reject) => {
    const transport = url.protocol === "https:" ? https : http;
    const req = transport.request(url, { method: options.method, headers: options.headers }, (res) => {
      const chunks: Buffer[] = [];
      res.on("data", (c: Buffer) => chunks.push(c));
      res.on("end", () => {
        try {
          resolve(JSON.parse(Buffer.concat(chunks).toString("utf8")) as T);
        } catch (e) {
          reject(e);
        }
      });
    });
    req.on("error", reject);
    if (options.body) req.write(options.body);
    req.end();
  });
}
```

## The BFF route — attach the token, refresh on `3001`

A catch-all proxy for user reads. It injects the access token, and if the API
says the token expired (`error_code` `3001`), it refreshes once and retries —
transparently to the browser.

`app/bff/[...path]/route.ts`:

```ts
import { randomUUID } from "node:crypto";
import { cookies } from "next/headers";
import { type NextRequest, NextResponse } from "next/server";
import { requestApiJson } from "@/lib/api/server";
import { refreshSession } from "@/lib/auth/session";

const ERR_FORBIDDEN = 3001; // access token expired

export async function GET(request: NextRequest, ctx: { params: Promise<{ path: string[] }> }) {
  const requestId = request.headers.get("x-request-id") ?? randomUUID();
  let access = (await cookies()).get("oa_access")?.value;
  if (!access) {
    const refreshed = await refreshSession(requestId);
    if (refreshed.ok) access = refreshed.accessToken;
  }
  if (!access) {
    return NextResponse.json({ success: false, data: null, message: "Sign in first" }, { status: 403 });
  }

  const apiPath = `/${(await ctx.params).path.join("/")}`;
  const data = await requestApiJson<unknown>(apiPath, { authorization: `Bearer ${access}`, requestId });

  if (!data.success && data.error_code === ERR_FORBIDDEN) {
    const refreshed = await refreshSession(requestId);
    if (refreshed.ok) {
      return NextResponse.json(
        await requestApiJson<unknown>(apiPath, { authorization: `Bearer ${refreshed.accessToken}`, requestId }),
      );
    }
  }
  return NextResponse.json(data);
}
```

Only proxy **user-scoped reads** through the BFF (`/me`, `/billing/balance`,
`/billing/transactions`). **Never** proxy `/authorize` or `/charge` here — those
use the App API key and must run in dedicated server routes (see
[metering-and-billing.md](metering-and-billing.md)), never reachable with a user
cookie.

## Env

All four are **server-only** — never `NEXT_PUBLIC_*`. `lib/env/server.ts`:

```ts
import "server-only";

export function getServerEnv() {
  return {
    OS_ACCOUNTS_URL: process.env.OS_ACCOUNTS_URL ?? "http://localhost:3000",
    OS_ACCOUNTS_API_URL: process.env.OS_ACCOUNTS_API_URL ?? "http://127.0.0.1:3001",
    APP_ORIGIN: process.env.APP_ORIGIN ?? "http://localhost:3002",
    OS_ACCOUNTS_CLIENT_ID: process.env.OS_ACCOUNTS_CLIENT_ID ?? "", // ocl_… — safe identifier
    OS_ACCOUNTS_APP_API_KEY: process.env.OS_ACCOUNTS_APP_API_KEY ?? "", // osk_… — never commit a real one
  };
}
```

| Var | Purpose | Example |
|---|---|---|
| `OS_ACCOUNTS_URL` | OS Accounts site (login redirect + top-up link) | `https://accounts.opensoftware.co` |
| `OS_ACCOUNTS_API_URL` | Public API origin (BFF + server-to-server target) | `https://api.accounts.opensoftware.co` |
| `APP_ORIGIN` | Your app's own public origin (builds `redirect_uri`) | `https://yourapp.com` |
| `OS_ACCOUNTS_CLIENT_ID` | OAuth client id created in the Admin console | `ocl_…` |
| `OS_ACCOUNTS_APP_API_KEY` | Your App API key for `/authorize` + `/charge` | `osk_…` (secret) |

The `import "server-only"` guard makes the build fail if this module is ever
imported into a client component — a cheap insurance against leaking the key.

## The envelope

Every non-public API response is:

```jsonc
{
  "data": { /* the payload, or null on error */ },
  "success": true,
  "error_code": 4301,   // present on failure; an application code, not the HTTP status
  "message": "Insufficient credits"
}
```

Branch on `success` / `error_code`, not just the HTTP status. Application error
codes are **banded integers** that map to an HTTP status:

| Band | HTTP | Meaning (examples) |
|---|---|---|
| `1001–1999` | 404 | Not found |
| `2001–2999` | 400 | Bad request / validation |
| `3001–3999` | 403 | Forbidden — **`3001` = access token expired → refresh + retry** |
| `4001–4099` | 409 | Conflict (e.g. idempotency-key shape mismatch) |
| `4101–4199` | 410 | Gone (e.g. consumed/expired auth code) |
| `4201–4299` | 422 | Unprocessable (e.g. payment needs customer action) |
| `4301–4399` | 402 | Billing — **`4301` = insufficient credits** |
| `4401–4499` | 429 | Rate limited |
| `5000–5099` | 500 | Internal |
| `9001–9999` | 501 | Not implemented |

Handle `3001` by refreshing (the BFF does it for you), `4301` by prompting a
top-up, and `4401` by backing off.
