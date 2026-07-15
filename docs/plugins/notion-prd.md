# PRD: Notion plugin

- **Mode:** CEO
- **Rank:** 6 of 10
- **Score:** 77/100
- **Date:** 2026-07-13
- **Status:** Proposed; tracked by JUN-283

## Thesis

Notion is the strongest knowledge-base complement to June's private note graph.
The plugin should bridge ephemeral meeting context and durable team knowledge:
prepare from selected pages and databases, then publish a reviewed decision
record, project update, or action list back to the right location.

It ranks below the universal and ecosystem plugins because many users do not
use Notion and its REST OAuth token exchange creates a privacy-architecture
constraint. Per [ADR 0025](../adr/0025-notion-oauth-via-hosted-mcp.md), v1
uses Notion hosted MCP's OAuth path rather than Notion REST public OAuth or
pasted tokens. It ranks above software delivery tools because its workflows
apply across roles.

## Customer and problem

Teams intend Notion to be the source of truth, but meeting outcomes arrive late,
inconsistently, or not at all. Before a meeting, people search pages manually.
Afterward, they translate notes into a page or database row and lose source
traceability. Broad workspace indexing solves search by increasing data copies.

## Product promise

Connect Notion with OAuth, then choose the Notion pages June may expose to
Hermes. Prepare and publish from those bounded sources while local June notes
are not copied to Notion unless the user explicitly sends content there. V1
avoids pasted tokens and does not embed a Notion REST client secret; it uses
Notion hosted MCP's OAuth path while keeping OpenSoftware servers out of the
Notion credential path. Notion hosted MCP grants can cover the authorizing user's full
Notion permission graph, so Phase 0 must prove each exposed tool can be scoped
before the hosted MCP call or the product promise must be narrowed. Notion
content or note-derived draft context included in prompts still follows the
user's selected model path.

## V1 experience

- Connect from Plugins with Notion OAuth, then select roots in June's own picker.
- Search selected pages/data sources and read a page on demand, only for hosted
  MCP tools whose requests can be constrained before the remote call.
- Link a June note to a Notion page without copying the entire note.
- Draft a decision record, project update, or action database entry from a
  meeting.
- Preview the destination, parent, properties, and content before creation or
  update.
- Disconnect and remove every Notion capability from the runtime.

## Scope

### V1

- Search, page/data-source metadata, bounded page/block read, comments read.
- Create a page under an approved parent.
- Append/update a narrowly targeted block range or database properties.
- Meeting brief, decision record, and project-update skills.
- Local polling while awake for explicitly followed pages if feasible.

### Later

- Multiple workspaces, broad sync, comments/actions, files, team-wide install,
  and webhook-triggered routines through an approved away-mode relay.

## Non-goals

- Replacing Notion search or editing UI.
- Mirroring an entire workspace into June or OpenSoftware.
- Accessing pages outside the connected Notion account's accessible graph.
  Hosted MCP grants may cover the authorizing user's full Notion permission
  graph. June-selected roots are a narrower product boundary only for tools that
  can be constrained before the hosted MCP call; tools that cannot be scoped are
  out of scope or require a narrower product promise.
- Autonomous publishing at launch.
- Treating every database schema as a generic spreadsheet.

## Packaging

- Required connector: Notion.
- Skills: meeting-to-decision, project update, research brief, page maintenance.
- Templates: decision log, weekly project status, customer meeting recap.
- Composition: Google/Microsoft calendar and Slack context can feed a draft;
  Notion remains the explicit publication destination.

## Privacy and trust

Notion public REST OAuth requires a client secret for token exchange, refresh,
and revocation, so June v1 does not use that flow. Per [ADR 0025](../adr/0025-notion-oauth-via-hosted-mcp.md),
June uses Notion hosted MCP's OAuth path instead of pasted tokens, an embedded
REST client secret, or a June API OAuth broker. OpenSoftware servers hold no
Notion credential and OpenSoftware infrastructure is not in the Notion connector
data path; Notion's own hosted MCP service processes Notion connector requests
as part of Notion's service. This
claim covers token custody and connector calls, not model inference; Notion
content included in prompts follows the user's selected model path.

Notion content is untrusted input. Provider permissions define the maximum graph;
June-selected roots are a hard product boundary only where Phase 0 proves June
can constrain the hosted MCP call before Notion processes out-of-root content.
All creates and updates require approval in v1, with exact destination and diff
visible.

## Business model

OAuth connection, bounded reads, and approved publishing are Hobby if Phase 0
confirms the hosted MCP path preserves the privacy baseline. Triggered routines
and cross-plugin publishing flows are Pro.

## Success measures

| Metric | Target |
| --- | ---: |
| Connections selecting at least one page root | 85% |
| Weekly connected users reading or publishing | 30% |
| Drafts approved and published | 60% |
| Published pages requiring a manual structural repair | under 5% |
| Out-of-root content exposed to Hermes through June facade tools | 0 successful |

## Risks and gates

- Hosted MCP Phase 0 verification is the first gate: OAuth, tool allowlisting,
  Rust-only remote MCP access, approval-before-write enforcement, and pre-call
  selected-resource scoping must work from June's Tauri desktop context.
- Notion APIs expose block trees and evolving data-source schemas; lossless
  editing is not a credible v1 promise.
- Webhooks require a public endpoint and can arrive out of order; local v1 does
  not quietly depend on them.
- Page content can instruct the model to publish or disclose unrelated data.

## Decision requested

Proceed under the ADR 0025 OAuth-through-hosted-MCP decision with a bounded-page
Notion plugin for read-on-demand and approved publish; defer webhook routines.

## Sources

- [Notion public connection authorization](https://developers.notion.com/guides/get-started/authorization)
- [Notion public connections](https://developers.notion.com/guides/get-started/public-connections)
- [Notion hosted MCP overview](https://developers.notion.com/guides/mcp/overview)
- [Notion MCP client guide](https://developers.notion.com/guides/mcp/build-mcp-client)
- [Notion MCP security best practices](https://developers.notion.com/guides/mcp/mcp-security-best-practices)
- [Notion webhooks](https://developers.notion.com/reference/webhooks)
