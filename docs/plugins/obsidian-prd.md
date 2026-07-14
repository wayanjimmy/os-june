# PRD: Obsidian plugin

**Owner:** CEO · **Date:** 2026-07-14 · **Status:** Proposed for product gate
**Companion doc:** [obsidian-implementation-plan.md](obsidian-implementation-plan.md)

## Decision summary

Build Obsidian as a separate first-party local plugin only if the product gate confirms that graph-aware vault workflows deserve a surface distinct from Documents. Obsidian must not be modeled as a connector: it is a local filesystem capability, not a third-party account with OAuth, scopes, token custody, or refresh lifecycle.

Recommended v1:

- macOS first.
- One selected vault.
- Markdown notes only.
- Read tools: search notes, get note, list tags, and get backlinks.
- Write tools: create note and append note.
- Every write approved with an exact diff.
- No routines or autonomous writes.
- No Canvas, daily-note automation, rename, move, delete, attachment reads, or bulk link rewriting.
- No note content persisted in SQLite in v1.
- No June API change or deploy.

## Why this is separate from Documents

Documents creates local artifacts managed by June and exports them through an explicit save boundary. Obsidian operates on an existing user-owned vault whose files remain authoritative. A separate plugin is justified only when June uses vault semantics that plain Markdown export cannot provide:

- Search and synthesize an existing personal knowledge graph.
- Convert a June note into a durable vault note with links, tags, and properties.
- Add a project update or decision to an existing vault note.
- Find backlinks and related notes before preparing a meeting.
- File research into an established folder and naming system.

If dogfood shows users mostly want generic `.md` creation, this should fold into Documents instead.

## Product gate

Before implementation, answer:

1. What share of June's target users actively uses Obsidian or an Obsidian-compatible Markdown vault?
2. Are users asking for retrieval, filing, or ongoing updates?
3. Is read-only access valuable enough for activation?
4. Will users approve every write, or is the friction too high?
5. Which ranked plugin workstream, if any, does Obsidian displace?
6. Should the listing be named "Obsidian", "Obsidian vault", or "Markdown vault with Obsidian support"?
7. Is macOS-only v1 acceptable?
8. Must the selected folder contain `.obsidian/`? Recommended: yes.
9. Should writes be a separate opt-in after read access? Recommended: yes.
10. Are routines excluded from v1? Recommended: yes.

Gate artifacts:

- Portfolio score using the rubric in [portfolio.md](portfolio.md).
- V1 support matrix.
- Threat model.
- Privacy copy aligned with the actual inference path.
- ADR for the local vault grant and broker boundary.
- `CONTEXT.md` entry for local vault grant if the term becomes canonical.

## User experience

The plugin detail screen should show:

- What June can read.
- What June can change.
- That vault file reads and writes stay on this device.
- That selected content may leave the device for model inference unless a local model is selected.
- Supported and unsupported formats.
- Selected vault name and note count.
- Health state and last successful scan.
- Write setting, off by default.
- Rebuild index and remove vault access actions.

Use "Select vault", "Vault selected", and "Remove vault access". Avoid "Connect account" because no provider account is involved.

## Success measures

- Median time from selecting a vault to first successful search.
- Share of selected vaults that index without warnings.
- Search success rate and latency by vault size.
- Proposed write approval rate.
- Conflict and post-write correction rate.
- Weekly users completing a vault-backed task.
- Zero writes outside the selected root.
- Zero silent overwrites.

## Explicit deferrals

- Multiple vaults.
- Daily-note creation.
- Canvas read or writes.
- Attachment content reads.
- Typed frontmatter mutation.
- Rename, move, delete, and bulk link rewriting.
- Routines or unattended writes.
- Persistent full-text search cache unless benchmarks require it.

Rename, move, delete, and bulk link rewriting should be planned together because they are a multi-file transaction problem and materially riskier than create and append.
