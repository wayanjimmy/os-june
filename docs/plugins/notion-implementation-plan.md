# Implementation plan: Notion plugin

- **Mode:** CTO
- **Date:** 2026-07-13
- **Status:** Proposed; Phase 0A auth decision recorded in [ADR 0025](../adr/0025-notion-oauth-via-hosted-mcp.md)
- **PRD:** [notion-prd.md](notion-prd.md)
- **Issue:** JUN-283

## Technical objective

Expose selected Notion content through read and action MCP servers, with the
authorized page graph enforced in Rust and all writes parked for approval.

## Phase 0: auth boundary

Resolved by [ADR 0025](../adr/0025-notion-oauth-via-hosted-mcp.md), which
supersedes [ADR 0024](../adr/0024-notion-local-token-custody.md). Notion's REST
public OAuth flow requires HTTP Basic authentication with a client id and client
secret during code exchange, refresh, and revocation. A desktop binary cannot
protect that secret, and the REST OAuth surface does not provide a public-client
PKCE path. June therefore rejects embedded REST secrets, a generic OpenSoftware
token vault, and a June API OAuth broker for v1.

V1 is OAuth-first through Notion hosted MCP. June does not ask users to paste
Notion tokens. Phase 0 must verify that the hosted MCP OAuth flow works from
June's Tauri desktop context, that hosted MCP token material and any dynamic
client registration material can be stored only in Keychain, and that June can
wrap the hosted MCP with local governance: Rust-only remote MCP access, tool
allowlisting, approval-before-write, selected-resource state, output caps, safe
errors, and Notion-content-as-untrusted guidance. OpenSoftware servers hold no
Notion credential and June API is not in the Notion credential path. Notion's own
hosted MCP service is in the Notion connector request path and processes Notion
connector requests as part of Notion's service.

Hermes must never connect directly to Notion hosted MCP. The only allowed shape
is Hermes -> local June facade MCP -> Rust-hosted remote MCP client ->
`https://mcp.notion.com/mcp`. The hosted MCP URL, hosted OAuth token, refresh
token, and dynamic client registration material are never registered in Hermes
config and are never available to Python MCP servers or local MCP processes.

If Phase 0 shows that hosted MCP cannot support local token custody, reliable
tool allowlisting, approval-before-write enforcement, or pre-call enforcement of
selected-resource boundaries for the planned tools, implementation stops and the
architecture must be revisited before code ships.

## Proposed local facade servers

| Server | Tools |
| --- | --- |
| `june_notion` | Hosted-MCP-backed read tools verified in Phase 0, expected to cover `search_pages`, `get_page`, `list_data_source`, `query_data_source`, `list_comments` |
| `june_notion_actions` | Hosted-MCP-backed action tools verified in Phase 0, expected to cover `create_page`, `update_page_properties`, `append_blocks`, `update_block` |

June should expose local facade tool names that remain stable even if Notion's
hosted MCP tool names differ. Tool results must preserve, when available, page,
block, data-source ids, parent ids, canonical URLs, last-edited time, property
schema, pagination cursor, and a compact content representation. Full block
trees are bounded by depth, count, and bytes. If hosted MCP responses do not
provide enough stable identifiers or metadata for a v1 tool, that tool is out of
scope until the architecture is revisited.

## Authorization boundary

The hosted MCP OAuth grant and Notion permissions determine the maximum
accessible graph and may include the authorizing user's full Notion workspace
access. June-selected roots define a narrower product boundary only for tools
whose hosted MCP requests can be constrained before Notion processes
out-of-root content. Phase 0 must verify which stable resource identifiers,
parent metadata, and tool parameters hosted MCP exposes. Rust stores
June-selected roots as authority and treats cached metadata as non-authoritative.
Where hosted MCP cannot support pre-call ancestry, parent, or parameter
restriction for a read/write, June must narrow the exposed tool surface or
change the product promise. Output filtering may limit what reaches Hermes, but
it is not an access-control substitute because hosted MCP has already processed
the remote content. Writes additionally require an approved parent within the
current selected graph.

## State and events

- Hosted MCP OAuth token material and dynamic client registration material in
  Keychain only.
- Workspace/account id, selected roots, hosted MCP capabilities/tool inventory,
  and health in SQLite.
- No page bodies or database row corpus at rest.
- V1 freshness is live hosted-MCP fetch plus optional bounded polling of
  user-followed pages if the hosted tool surface supports it. Provider webhooks
  require public HTTPS and belong to away mode.

## Write model

- Create is preferred over broad in-place transformation.
- Updates operate on explicit page/block ids and include last-edited/version
  material where available.
- Approval shows workspace, destination breadcrumb, operation, property diff,
  and rendered content preview.
- A preflight re-reads destination state before commit. Stale state returns a
  conflict instead of overwriting.
- Autonomous mode is deferred.

## Delivery slices after Phase 0

1. **Hosted MCP spike (1 week):** OAuth from Tauri, Keychain custody for tokens
   and dynamic client registration, official-endpoint discovery pinning, tool
   inventory, tool allowlisting, direct-Hermes-access exclusion, write
   interception, pre-call selected-resource feasibility, rate-limit/error
   behavior, and privacy-copy verification.
2. **Connection shell (1 week):** workspace state, selected roots, disconnect
   and revocation guidance, health, plugin detail.
3. **Read path (2 weeks):** search, page/block read, data-source query,
   comments, limits and pagination through the local facade.
4. **Approved create (1 week):** create page with exact-parent approval.
5. **Targeted update (1-2 weeks):** properties and bounded block operations with
   conflict preflight where hosted MCP exposes enough state.
6. **Skills and rc (1 week):** decision/project templates, metrics, runbook.

## Verification

- Hosted MCP OAuth, dynamic client registration, Streamable HTTP `/mcp`,
  reconnect, disconnect and revocation guidance, removed-page-access, workspace
  removal, tool inventory drift, and partial capability matrix.
- Block-tree property tests for depth, pagination, unsupported types, mentions,
  embeds, equations, and files.
- Boundary tests that forged page/parent ids cannot escape authorized access.
- Conflict/idempotency tests for create and update around retries and restarts.
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
