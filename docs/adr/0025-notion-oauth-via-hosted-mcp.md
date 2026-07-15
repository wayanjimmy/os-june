---
status: accepted
date: 2026-07-15
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
paste Notion access tokens into June. We still reject embedding a Notion REST
client secret in the desktop app, because a shipped binary cannot protect that
secret. We also reject a June API OAuth broker for v1 because it would put
OpenSoftware infrastructure in the Notion credential exchange path and change
the local-mode privacy claim.

Notion's hosted MCP service is a separate surface from Notion REST public OAuth.
It supports OAuth 2.0 with PKCE for MCP clients and is documented as the
recommended maintained MCP path. Using it changes the connector request path:
June no longer calls Notion REST directly for v1 Notion tools. Instead, June
connects to Notion's hosted MCP service, and that service calls Notion as part
of Notion's own product boundary.

Notion hosted MCP grants are broad. Notion's MCP documentation describes the
connected AI system as having the authorizing user's Notion access, including
full workspace access where the user has it. June-selected roots are therefore a
June product boundary layered under a provider grant whose maximum graph is the
authorizing user's Notion permission graph.

## Decision

June's Notion connector v1 supersedes ADR 0024 and uses an OAuth-first
connection flow through Notion hosted MCP, subject to a Phase 0 verification
spike before full connector implementation.

- June will not ask users to paste Notion tokens in the normal product flow.
- June will not embed a Notion REST public-connection client secret in the
  desktop app.
- June will not use June API as a confidential Notion OAuth broker in v1.
- June will connect to Notion hosted MCP using the provider's OAuth/PKCE flow
  if Phase 0 confirms it works from June's Tauri desktop context.
- OAuth token material and any hosted-MCP dynamic client registration material
  are stored only in the OS Keychain under a Notion-specific service
  (`co.opensoftware.june.notion`, with `co.opensoftware.june-dev.notion` for
  debug builds). No Notion credential material is stored in SQLite, logs, issue
  reports, Hermes workspaces, MCP server environment, or command arguments.
- OpenSoftware servers still hold no Notion credential. June API is not in the
  Notion connector credential path. Notion's own hosted MCP service is in the
  Notion connector request path.
- Hermes must never connect directly to Notion hosted MCP. The only allowed
  shape is: Hermes -> local June facade MCP -> Rust-hosted remote MCP client ->
  `https://mcp.notion.com/mcp`. The hosted MCP URL, hosted OAuth token, refresh
  token, and dynamic client registration material are never registered in Hermes
  config and are never available to Python MCP servers or local MCP processes.
- June wraps the hosted MCP behind local connector governance wherever the MCP
  protocol and hosted tool surface allow it: local account state, runtime
  registration, June-owned tool allowlists, action approval, selected-resource
  state, content-size limits, safe errors, and Notion-content-as-untrusted
  guidance.
- June-selected roots are a product requirement only for tools that can be
  constrained before the hosted MCP call or otherwise proven not to request
  out-of-root content. Output filtering after a broad hosted search or fetch is
  not sufficient to claim an access boundary; the connector has already asked
  Notion hosted MCP to process the content.
- Write tools remain approval-only in v1. Autonomous Notion publishing is
  deferred.
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

Before implementation proceeds beyond a spike, verify and document:

1. The hosted MCP OAuth flow works from the Tauri desktop app, including the
   callback URL shape, PKCE, cancellation, timeout, denial, token refresh,
   reconnect behavior, and whether dynamic client registration is required.
2. Token material and dynamic client registration material can be stored in
   Keychain and kept out of SQLite, logs, issue reports, Hermes home, MCP
   process environment, and command arguments.
3. OAuth discovery is pinned to the official Notion hosted MCP protected
   resource (`https://mcp.notion.com/mcp`). Discovered issuers and
   authorization, token, registration, and revocation endpoints are validated
   against an explicit Notion-owned allowlist established and recorded by Phase
   0 before credentials are sent.
4. June can enumerate hosted MCP tools and expose only an approved subset to
   Hermes through local facade tools. Unknown hosted tools, schema drift, or
   unclassified tools fail closed.
5. Hermes cannot connect directly to Notion hosted MCP and all hosted
   `tools/call` operations cross the Rust governance choke point.
6. June can identify which hosted tools mutate Notion state through a
   June-owned allowlist that maps exact hosted tool names and schemas to read or
   mutating behavior, and can park every mutating operation for June approval
   before execution.
7. Hosted MCP tool requests and responses expose enough stable page,
   data-source, URL, parent, or workspace metadata to enforce selected-resource
   boundaries before remote calls for each exposed tool and to render approval
   previews. Tools that cannot be constrained pre-call are excluded or the
   product promise is narrowed.
8. Hosted MCP rate limits, error shapes, token expiry, refresh behavior,
   refresh-token rotation, dynamic-registration reuse, revocation behavior, and
   connected-app removal behavior are known well enough to implement safe
   retries and reconnect states.
9. Streamable HTTP `/mcp` behavior is verified, including initialization,
   session/reconnect behavior, cancellation, response-size limits, timeouts, and
   tool inventory drift. SSE fallback remains optional and must be verified
   separately if used.
10. The threat model and user-facing copy can accurately describe the path:
    device -> Notion hosted MCP -> Notion, with OpenSoftware outside that path,
    while also disclosing that the provider grant's maximum graph follows the
    authorizing user's Notion access.

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
