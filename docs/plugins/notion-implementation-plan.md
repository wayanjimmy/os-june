# Implementation plan: Notion plugin

- **Mode:** CTO
- **Date:** 2026-07-13
- **Status:** Proposed; Phase 0A auth decision recorded in [ADR 0024](../adr/0024-notion-local-token-custody.md)
- **PRD:** [notion-prd.md](notion-prd.md)
- **Issue:** JUN-283

## Technical objective

Expose selected Notion content through read and action MCP servers, with the
authorized page graph enforced in Rust and all writes parked for approval.

## Phase 0: auth boundary

Resolved by [ADR 0024](../adr/0024-notion-local-token-custody.md). Notion's
REST public OAuth flow requires HTTP Basic authentication with a client id and
client secret during code exchange, refresh, and revocation. A desktop binary
cannot protect that secret, and the REST OAuth surface does not provide a
public-client PKCE path. June therefore rejects embedded secrets, a generic
OpenSoftware token vault, and a June API OAuth broker for v1.

V1 uses a local token-paste connection flow: the user creates or selects a
Notion internal connection token, or an advanced personal access token, and
pastes it into June. Internal connections see pages and data sources shared with
that connection in Notion. PATs act as the creating user and are broader, so
June presents them as an advanced fallback and narrows use through
June-selected roots. June stores the token only in the Keychain, sends Notion
REST calls directly from the device, and keeps June API out of the Notion
connector data path. Disconnect deletes local credentials; server-side
invalidation is done by removing the internal connection or PAT in Notion.

This preserves the local-mode credential claim at the cost of onboarding
friction and no Notion OAuth page picker. Notion hosted MCP with PKCE remains a
future option, but it changes the request path and requires its own ADR and
threat model update before implementation.

## Proposed servers

| Server | Tools |
| --- | --- |
| `june_notion` | `search_pages`, `get_page`, `list_data_source`, `query_data_source`, `list_comments` |
| `june_notion_actions` | `create_page`, `update_page_properties`, `append_blocks`, `update_block` |

Tool results preserve page/block/data-source ids, parent ids, canonical URLs,
last-edited time, property schema, pagination cursor, and a compact content
representation. Full block trees are bounded by depth, count, and bytes.

## Authorization boundary

Notion's token capabilities and provider permissions determine the maximum
accessible graph. Internal connections are additionally bounded by which pages
or data sources the user shares with that connection in Notion. PATs follow the
creating user's Notion permissions and can be broader. Because the v1
token-paste flow does not use Notion's OAuth page picker, June must provide its
own post-connect resource picker. Rust stores only June-selected roots as
authority and treats cached metadata as non-authoritative. It verifies ancestry
live against current provider state and treats provider 403/404 as permission
outcomes, never as cached proof of access. Writes additionally require an
approved parent within the current selected graph.

## State and events

- Notion local token in Keychain only.
- Workspace/account id, bot id when available, selected roots, capabilities,
  and health in SQLite.
- No page bodies or database row corpus at rest.
- V1 freshness is live fetch plus optional bounded polling of user-followed
  pages. Provider webhooks require public HTTPS and belong to away mode.

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

1. **Connection shell (1 week):** workspace state, selected roots, disconnect
   and revocation guidance, health, plugin detail.
2. **Read path (2 weeks):** search, page/block read, data-source query, comments,
   limits and pagination.
3. **Approved create (1 week):** create page with exact-parent approval.
4. **Targeted update (1-2 weeks):** properties and bounded block operations with
   conflict preflight.
5. **Skills and rc (1 week):** decision/project templates, metrics, runbook.

## Verification

- Auth, reconnect, disconnect and revocation guidance, removed-page-access,
  workspace removal, and partial capability matrix.
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

Any backend-held secret or token, public webhook intake, or provider content
relay is a new trust boundary and requires an ADR before shipping.
