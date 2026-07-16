---
status: accepted
date: 2026-07-16
supersedes: 0024-notion-local-token-custody
---

# Notion connector is OAuth-first through Notion hosted MCP

## Context

ADR 0024 chose a user-supplied local token for Notion v1 because Notion REST
public OAuth requires a confidential client secret for token exchange, refresh,
and revocation. That choice preserved June's strict local REST connector shape,
but it created a high-friction setup where users had to create or choose a
Notion internal connection token or personal access token and paste it into
June.

The product direction has changed: Notion must be OAuth-first. Users should not
paste Notion tokens into June. We still reject embedding a Notion REST client
secret in the desktop app, because a shipped binary cannot protect that
secret. We also reject a June API OAuth broker for v1 because it would put
OpenSoftware infrastructure in the Notion credential exchange path and change
the local-mode privacy claim.

Notion's hosted MCP service is a separate surface from Notion REST public OAuth.
It supports OAuth 2.0 with PKCE for MCP clients and is documented as the
recommended maintained MCP path. The endpoint is
`https://mcp.notion.com/mcp` (Streamable HTTP). Using it changes the connector
request path: June no longer calls Notion REST directly for v1 Notion tools.
Instead, June's Rust shell acts as a standard MCP client connecting to Notion's
hosted MCP service, and that service calls Notion as part of Notion's own
product boundary.

Notion hosted MCP grants are broad. The connected AI system has the authorizing
user's Notion access, including full workspace access where the user has it.
June-selected roots are therefore a June product boundary layered under a
provider grant whose maximum graph is the authorizing user's Notion permission
graph.

A prior version of this ADR described the architecture as
"Hermes -> local June facade MCP -> Rust-hosted remote MCP client". The
clarified and simplified shape is: June's Rust shell is the standard remote MCP
client (using the official Rust MCP SDK, not hand-written transport), and a
thin in-process policy adapter sits between Hermes and that client. The adapter
is not a custom replacement for MCP; it is a narrow governance layer.

## Decision

June's Notion connector v1 supersedes ADR 0024 and uses an OAuth-first
connection flow through Notion hosted MCP, subject to a Phase 0 verification
spike before full connector implementation.

### Connection and credential custody

- June will not ask users to paste Notion tokens in the normal product flow.
- June will not embed a Notion REST public-connection client secret in the
  desktop app.
- June will not use June API as a confidential Notion OAuth broker in v1.
- June's Rust shell connects to Notion hosted MCP at `https://mcp.notion.com/mcp`
  using the provider's OAuth/PKCE flow, if Phase 0 confirms it works from June's
  Tauri desktop context.
- Production transport uses a reviewed and pinned Rust MCP SDK (`rmcp`) for
  protocol handling, session management, and Streamable HTTP. The existing spike
  code (`notion_mcp_oauth.rs`) is exploration-only and is not production
  transport: it lacks refresh implementation, re-registers on every connection,
  manually pins one protocol version, does not paginate `tools/list`, buffers
  response bodies without a byte cap, and returns only the first SSE data event.
- OAuth token material and any hosted-MCP dynamic client registration material
  are stored only in the OS Keychain under a Notion-specific service
  (`co.opensoftware.june.notion`, with `co.opensoftware.june-dev.notion` for
  debug builds). No Notion credential material is stored in SQLite, logs, issue
  reports, Hermes workspaces, MCP server environment, or command arguments.
- OpenSoftware servers still hold no Notion credential. June API is not in the
  Notion connector credential path. Notion's own hosted MCP service is in the
  Notion connector request path.

### Architecture: Rust is the MCP client, Hermes sees only curated tools

- Hermes must never connect directly to Notion hosted MCP. Hermes stores OAuth
  tokens as JSON in `HERMES_HOME/mcp-tokens` with no Keychain injection hook at
  June's pin, bypasses approval for regular MCP calls (per ADR 0016), has
  unreliable `hermes mcp login` requiring a PTY, and cannot guarantee
  content-free logs or strict response byte caps. These are documented in the
  hermes-gateway-gotchas.
- The only allowed shape is:

  ```text
  Hermes
    -> local June policy adapter (curated tools, no Notion credentials)
    -> in-process Rust MCP client (rmcp)
    -> https://mcp.notion.com/mcp
  ```

- The hosted MCP URL, hosted OAuth token, refresh token, and dynamic client
  registration material are never registered in Hermes config and are never
  available to Python MCP servers or local MCP processes.
- The policy adapter is a thin in-process governance layer, not a custom
  replacement for MCP. It does not reimplement OAuth discovery, Streamable HTTP
  sessions, JSON-RPC correlation, cancellation, or SSE parsing. Those are
  standard client concerns handled by the SDK.
- The adapter must retain these responsibilities:
  - **Deny-by-default inventory:** expose only exact approved tools and schemas.
  - **June-owned read/write classification:** do not rely on MCP annotations.
    Unknown tools or schema drift fail closed.
  - **Argument validation and selected-resource checks:** performed before
    remote `tools/call`.
  - **Approval:** every mutating operation parks before the Notion request.
  - **Output controls:** remote response byte cap, duration, event/count/depth
    limits, and final Hermes context cap.
  - **Content-free diagnostics:** only stable operation class, outcome,
    latency/size buckets, rate-limit state, and reconnect reason.
  - **Retry policy:** reads may retry safely; mutating calls must not be blindly
    retried after ambiguous timeout.

### Stable June tool names

- June exposes stable, June-defined tool names to Hermes even if Notion's hosted
  MCP tool names differ. Each June tool is a narrower June capability, not a
  transparent alias.
- Each June tool maps to an expected hosted tool name plus an accepted schema
  fingerprint. If the hosted inventory changes (tool inventory drift), the
  affected June tool is disabled until reviewed.
- The implementation plan's predicted tool names (`search_pages`, `get_page`,
  `append_blocks`, etc.) are placeholders. Notion currently exposes broader
  tools such as `notion-search`, `notion-fetch`, `notion-create-pages`, and
  `notion-update-page`. Phase 0 must capture the real inventory.

### Selected resources

- June-selected roots are a product requirement only for tools that can be
  constrained before the hosted MCP call or otherwise proven not to request
  out-of-root content. Output filtering after a broad hosted search or fetch is
  not sufficient to claim an access boundary; the connector has already asked
  Notion hosted MCP to process the content.
- `notion-search` can search the entire workspace and, with Notion AI, connected
  Slack, Drive, and Jira sources. It should be excluded unless its live schema
  proves June can constrain it before execution.
- `notion-fetch` accepts an arbitrary URL or ID. Post-fetch ancestry filtering is
  too late under the current privacy promise because hosted MCP has already
  processed the page.
- Phase 0 may conclude that v1 supports only selected exact pages and approved
  destinations, not an arbitrary descendant graph or workspace search. If
  selected-root search and traversal are non-negotiable and the hosted schemas
  cannot enforce them pre-call, hosted MCP is not viable for that product
  promise.

### Reads-first v1

- V1 ships read-only tools first. This avoids 80% of the policy adapter
  complexity (approval gating, ambiguous-timeout handling, idempotency,
  duplicate suppression, no-retry-after-ambiguous-write).
- The read-only adapter is: allowlist approved read tools, enforce output caps,
  and log content-free diagnostics. Hermes's existing `tools.include` filtering
  provides defense in depth.
- A local MCP policy endpoint is not needed on day one. It emerges when writes
  are added and a proper approval interception point is required.

### Writes deferred to v2

- Write tools remain approval-only. Autonomous Notion publishing is deferred.
- When writes are added: June-owned read/write classification (not MCP
  annotations), approval flow (park every mutating call before the network),
  ambiguous-write handling, idempotency, and no-blind-retry rules.
- The local MCP policy endpoint becomes the approval interception point at this
  stage.

### Gate condition

- If Phase 0 shows that hosted MCP cannot support local token custody, reliable
  tool allowlisting, approval-before-write enforcement, or pre-call enforcement
  of selected-resource boundaries for the planned tools, implementation stops
  and this ADR must be revisited before code ships.

## Consequences

- The onboarding experience becomes OAuth-first, matching the desired product
  flow and avoiding pasted tokens.
- The privacy copy changes. June can say OpenSoftware servers do not hold Notion
  credentials and OpenSoftware infrastructure is not in the Notion connector data
  path. June must also say Notion's hosted MCP service processes Notion
  connector requests as part of Notion's service. The old local REST claim,
  "device -> Notion REST", no longer applies to v1 Notion.
- The provider grant is broader than June-selected roots. Notion hosted MCP can
  see the authorizing user's accessible Notion graph when June calls hosted MCP;
  June can only expose a root-bounded product surface for tools whose parameters
  and metadata allow June to enforce that boundary before the call.
- June's enforcement boundary is narrower than with a local REST client. The
  Rust host can decide which hosted MCP tools are exposed and can gate calls it
  proxies, but it cannot change Notion hosted MCP's internal implementation.
  Phase 0 must prove that this is good enough for the v1 tool surface.
- June-selected roots remain a product requirement, but if hosted MCP cannot
  scope a read or write before the remote tool call, that tool is out of scope
  or the product promise must be narrowed. Output filtering may still be useful
  to limit what reaches Hermes, but it is not an access-control substitute.
- Notion REST direct integration remains a future option only if Notion later
  publishes a public-client REST OAuth flow or June accepts a new trust boundary
  through a separate ADR.
- ADR 0024 remains useful historical context for why direct REST OAuth and
  embedded secrets were rejected, but its token-paste v1 decision is superseded.

## Phase 0 verification requirements

Phase 0 must produce a go/no-go matrix, not merely a successful OAuth login.

### 1. OAuth lifecycle

- System-browser loopback PKCE from Tauri.
- Reuse dynamic client registration (do not re-register on every connection).
- Cancellation, denial, timeout, restart, refresh rotation, `invalid_grant`,
  connected-app removal, and reconnect.
- Sentinel verification that credentials appear only in Keychain.

### 2. Standards client

- Pin and exercise `rmcp` against `/mcp`.
- Initialization, paginated inventory, session reconnect, notifications,
  cancellation, no redirects, exact-origin pinning.
- Strict caps for JSON, SSE events, aggregate bytes, errors, and duration.

### 3. Inventory contract

- Capture complete live tool names, schemas, annotations, and representative
  outputs.
- Define approved hosted-name/schema fingerprints and June mappings.
- Demonstrate unknown tools and drift become unavailable.
- Classify writes using June policy, not annotations.

### 4. Selected-resource matrix

- For every candidate tool, state exactly how it is constrained before the
  remote call.
- Test forged page, data-source, parent, view, and task IDs.
- Exclude search/query/fetch paths that cannot prove scope.

### 5. Approval proof (when writes are added)

- Hermes sees only local curated tools.
- Every call crosses Rust.
- Denied and timed-out writes result in zero remote `tools/call`.
- Approved calls preserve the exact reviewed arguments.
- No automatic retry after an ambiguous write timeout.

### 6. Output and observability audit

- Seed sentinel titles, IDs, page text, queries, and token-like strings.
- Inspect Hermes home, logs, SQLite, support bundles, crash diagnostics,
  renderer IPC, and telemetry.
- Verify content appears only where functionally required as a tool result,
  never in diagnostics.
- Verify over-limit responses are rejected before unbounded allocation.

### 7. Provider behavior

- 401 and serialized refresh, 429/`Retry-After`, workspace-wide rate limits,
  revocation, async operations, and partial failures.
- Test that connected-source search cannot escape the Notion-only product
  boundary.

## Alternatives considered

### Keep ADR 0024 token paste

Rejected by the new product direction. It preserves a strict local REST path,
but the user experience is too high friction and users should not have to paste
Notion tokens into June.

### Notion REST public OAuth with an embedded secret

Rejected. A desktop binary cannot keep a Notion public-connection client secret
confidential. Anyone who extracts it could exchange, refresh, or revoke grants
for that public connection.

### June API confidential OAuth broker

Rejected for v1. It could support Notion REST public OAuth without embedding the
secret in the app, but OpenSoftware infrastructure would participate in the
credential exchange and could hold credential material capable of refreshing a
Notion grant. That is a different privacy claim and requires a separate ADR and
threat model.

### Direct Notion REST OAuth with PKCE only

Rejected under current Notion REST docs. The REST public OAuth surface requires
client-secret authentication at the token endpoints and does not document a
public desktop PKCE alternative. If Notion later adds one, this decision can be
superseded again.

### Hermes connects directly to Notion hosted MCP

Rejected. Hermes stores OAuth tokens as JSON in `HERMES_HOME/mcp-tokens`, not
Keychain. No storage-injection hook exists at June's pin. Regular MCP calls
bypass Hermes approval (per ADR 0016), so a mutating Notion call could reach the
network before June approves it. `hermes mcp login` requires a PTY, has
unreliable exit status, incomplete auth status, and no Windows flow. Hermes
cannot guarantee content-free MCP logs or strict response byte caps. Hermes tool
filtering controls visibility, not argument-level authorization. Patching Hermes
to add Keychain storage, universal interception, schema classification, output
accounting, and content-free logging would effectively move the same policy
adapter into a harder-to-maintain runtime fork.

### Full custom facade as originally described

Superseded by the simplified adapter approach. The prior version of this ADR
described a "local June facade MCP" that could be read as a custom replacement
for MCP. The clarified approach uses the official Rust MCP SDK for standard
protocol concerns and a thin policy adapter for June-specific governance only.
This reduces custom code while preserving all security boundaries.
