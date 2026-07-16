# Implementation plan: Notion plugin

- **Mode:** CTO
- **Date:** 2026-07-16
- **Status:** Proposed; auth decision recorded in [ADR 0025](../adr/0025-notion-oauth-via-hosted-mcp.md)
- **PRD:** [notion-prd.md](notion-prd.md)
- **Issue:** JUN-283

## Technical objective

Expose selected Notion content through a Rust-hosted MCP client connected to
Notion hosted MCP, with a thin policy adapter governing what Hermes sees. V1
ships read-only; writes are deferred to v2.

## Architecture

ADR 0025 supersedes ADR 0024. The connector shape is:

```text
Hermes
  -> local June policy adapter (curated tools, no Notion credentials)
  -> in-process Rust MCP client (rmcp)
  -> https://mcp.notion.com/mcp
```

June's Rust shell is the standard remote MCP client. Production transport uses
the official Rust MCP SDK (`rmcp`), not hand-written HTTP/SSE. The existing
spike (`notion_mcp_oauth.rs`) is exploration-only.

Hermes must never connect directly to Notion hosted MCP. The hosted MCP URL,
OAuth token, refresh token, and dynamic client registration material are never
registered in Hermes config. See ADR 0025 for the full rationale.

The policy adapter is a thin in-process governance layer, not a custom MCP
replacement. It does not reimplement OAuth discovery, Streamable HTTP sessions,
JSON-RPC correlation, cancellation, or SSE parsing. It owns: deny-by-default
inventory, June-owned read/write classification, argument validation and
selected-resource checks, approval gating (v2), output controls, content-free
diagnostics, and retry policy.

## Phase 0: go/no-go spike

Phase 0 must produce a go/no-go matrix, not merely a successful OAuth login.
See ADR 0025 for the full verification requirements. The seven areas:

1. **OAuth lifecycle** — system-browser loopback PKCE from Tauri, dynamic
   registration reuse, cancellation/denial/timeout/refresh
   rotation/`invalid_grant`/reconnect, sentinel Keychain verification.
2. **Standards client** — pin `rmcp` against `/mcp`, paginated inventory,
   session reconnect, strict byte/event/duration caps.
3. **Inventory contract** — capture live tool names/schemas/annotations,
   define approved fingerprints and June mappings, drift fails closed,
   June-owned write classification.
4. **Selected-resource matrix** — for each candidate tool, prove pre-call
   constraint; forge-test page/parent/data-source IDs; exclude unconstrained
   search/query/fetch.
5. **Approval proof** (deferred to v2) — denied/timed-out writes produce zero
   remote `tools/call`; no auto-retry after ambiguous write timeout.
6. **Output and observability audit** — sentinel data in logs/SQLite/crash
   diagnostics/telemetry; over-limit responses rejected before unbounded
   allocation.
7. **Provider behavior** — 401 refresh, 429/`Retry-After`, revocation,
   connected-source search boundary.

If Phase 0 shows hosted MCP cannot support local token custody, reliable tool
allowlisting, or pre-call selected-resource enforcement, implementation stops
and the architecture must be revisited.

## V1: read-only tools

V1 ships read-only. The read-only adapter is minimal:

- Allowlist approved read tools (deny-by-default).
- Enforce output caps (byte, count, depth, duration).
- Content-free logging (operation class, outcome, latency/size buckets).
- Hermes `tools.include` filtering as defense in depth.

A local MCP policy endpoint is not needed on day one. Rust owns the MCP client
connection and intercepts at the transport level. The local endpoint emerges
when writes are added.

### Expected v1 tool surface

Phase 0 captures the real hosted MCP tool inventory. Current placeholders
(not real names):

| June tool | Likely hosted tool | Notes |
| --- | --- | --- |
| `june_notion_get_page` | `notion-fetch` or similar | Exact page ID only; must prove pre-call scope |
| `june_notion_list_comments` | TBD | Exact page ID only |
| `june_notion_query_data_source` | TBD | Exact data source ID only |

Tools like `notion-search` that accept unbounded queries are excluded unless
Phase 0 proves pre-call constraint is possible.

## V2: approved writes

When writes are added:

- June-owned read/write classification (do not trust MCP annotations).
- Approval flow: every mutating operation parks before the network call.
- Local MCP policy endpoint becomes the approval interception point.
- Ambiguous-write handling, idempotency, no-blind-retry.
- Approval preview shows workspace, destination breadcrumb, operation,
  property diff, and rendered content preview.
- Preflight re-reads destination state before commit. Stale state returns a
  conflict instead of overwriting.
- Create is preferred over broad in-place transformation.
- Autonomous mode is deferred.

## Stable June tool names

June exposes stable, June-defined tool names to Hermes even if Notion's hosted
MCP tool names differ. Each June tool maps to an expected hosted tool name plus
an accepted schema fingerprint. If the hosted inventory changes, the affected
June tool is disabled until reviewed.

## Authorization boundary

The hosted MCP OAuth grant and Notion permissions determine the maximum
accessible graph. June-selected roots define a narrower product boundary only
for tools whose hosted MCP requests can be constrained before Notion processes
out-of-root content. Rust stores June-selected roots as authority and treats
cached metadata as non-authoritative.

Phase 0 may conclude that v1 supports only selected exact pages and approved
destinations, not an arbitrary descendant graph or workspace search.

## State and events

- Hosted MCP OAuth token material and dynamic client registration material in
  Keychain only.
- Workspace/account id, selected roots, hosted MCP capabilities/tool inventory,
  and health in SQLite.
- No page bodies or database row corpus at rest.
- V1 freshness is live hosted-MCP fetch. Provider webhooks require public HTTPS
  and belong to away mode.

## Delivery slices

1. **Phase 0 spike (1 week):** OAuth from Tauri via `rmcp`, Keychain custody,
   registration reuse, tool inventory capture, selected-resource feasibility
   matrix, rate-limit/error behavior, observability sentinel audit. Go/no-go
   decision.
2. **Connection shell (1 week):** workspace state, selected roots, disconnect
   and revocation guidance, health, plugin detail.
3. **Read path (2 weeks):** approved read tools through the policy adapter with
   output caps and content-free logging.
4. **Skills and rc (1 week):** decision/project templates, metrics, runbook.
5. **Approved create (v2, 1 week):** create page with exact-parent approval.
6. **Targeted update (v2, 1-2 weeks):** properties and bounded block operations
   with conflict preflight.

## Verification

- Hosted MCP OAuth, dynamic client registration, Streamable HTTP `/mcp`,
  reconnect, disconnect and revocation, tool inventory drift, partial
  capability matrix.
- Boundary tests that forged page/parent ids cannot escape authorized access.
- Output cap tests: over-limit responses rejected before unbounded allocation.
- Content-free observability audit with sentinel data.
- Injection corpus in page title, rich text, comments, URLs, code blocks,
  database properties, and embedded content.
- Live workspace walkthrough with private/shared pages and schema changes.

## Rollout

Internal workspace, selected external workspaces, rc, stable. Use a provider
kill switch and content-free telemetry. Publish supported block/property types
and render unsupported content explicitly instead of dropping it.

## ADR threshold

Any OpenSoftware-held Notion secret or token, direct Notion REST OAuth,
public webhook intake, or non-Notion provider content relay is a new trust
boundary and requires an ADR before shipping.
