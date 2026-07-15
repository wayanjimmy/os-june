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
constraint. It ranks above software delivery tools because its workflows apply
across roles.

## Customer and problem

Teams intend Notion to be the source of truth, but meeting outcomes arrive late,
inconsistently, or not at all. Before a meeting, people search pages manually.
Afterward, they translate notes into a page or database row and lose source
traceability. Broad workspace indexing solves search by increasing data copies.

## Product promise

Choose the Notion pages June may use. Prepare and publish from those bounded
sources while local June notes are not copied to Notion unless the user
explicitly sends content there. V1 preserves local credential custody by using a
user-supplied Notion token stored in the Keychain, not Notion REST public OAuth.
Notion content or note-derived draft context included in prompts still follows
the user's selected model path.

## V1 experience

- Connect from Plugins by pasting a Notion internal connection token, or an
  advanced personal access token, then select roots in June's own picker.
- Search selected pages/data sources and read a page on demand.
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
- Accessing pages outside the Notion token's accessible graph or June-selected
  roots. For PATs, the accessible graph follows the creating user's Notion
  permissions and can be broader than an internal connection.
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
and revocation, so June v1 does not use that flow. Per [ADR 0024](../adr/0024-notion-local-token-custody.md),
users supply a Notion internal connection token or an advanced personal access
token, and June stores it only in the Keychain. Internal connections see pages
and data sources shared with that connection in Notion. PATs act as the creating
user and are broader, so June presents them as an advanced fallback and narrows
use through June-selected roots. OpenSoftware holds no Notion credential and is
not in the Notion connector data path. This claim covers token custody and
provider calls, not model inference; Notion content included in prompts follows
the user's selected model path.

Notion content is untrusted input. The user-selected page set and provider
permissions form a hard boundary. All creates and updates require approval in
v1, with exact destination and diff visible.

## Business model

Local reads and approved publishing are Hobby if the auth design preserves the
privacy baseline. Triggered routines and cross-plugin publishing flows are Pro.

## Success measures

| Metric | Target |
| --- | ---: |
| Connections selecting at least one page root | 85% |
| Weekly connected users reading or publishing | 30% |
| Drafts approved and published | 60% |
| Published pages requiring a manual structural repair | under 5% |
| Reads outside authorized page roots | 0 successful |

## Risks and gates

- Token-paste onboarding friction is the first gate.
- Notion APIs expose block trees and evolving data-source schemas; lossless
  editing is not a credible v1 promise.
- Webhooks require a public endpoint and can arrive out of order; local v1 does
  not quietly depend on them.
- Page content can instruct the model to publish or disclose unrelated data.

## Decision requested

Proceed under the ADR 0024 local-token auth decision with a bounded-page Notion
plugin for read-on-demand and approved publish; defer webhook routines.

## Sources

- [Notion public connection authorization](https://developers.notion.com/guides/get-started/authorization)
- [Notion public connections](https://developers.notion.com/guides/get-started/public-connections)
- [Notion webhooks](https://developers.notion.com/reference/webhooks)
