---
status: accepted
date: 2026-07-27
---

# Host text-file mutations are atomic and revision-aware

## Context

The host file tools previously let `write_file` overwrite an existing path and
let `patch_file` write a modified file directly. That blurred creation and
editing, could replace concurrent user changes, and could leave a partial file
after an interrupted write. Obsidian notes also commonly use CRLF or a UTF-8
BOM that a text edit must not silently normalize away.

ADR-0050's read and mutation path policy remains binding. This decision changes
mutation semantics within the paths that policy permits.

## Decision

Host text-file tools have three separate operations:

- `write_file` stages and syncs complete content beside the target, then
  publishes it with an operating-system no-replace operation. It never
  overwrites an existing file or exposes a partially written destination.
- `patch_file` replaces one unique exact occurrence in an existing file. For a
  uniform CRLF file only, LF or CRLF newlines in the narrow `before` and `after`
  arguments adapt to CRLF. For a uniform LF file they adapt to LF. Mixed line
  endings use raw exact matching and do not receive broad normalization.
- `replace_file` replaces all content only with the exact revision returned by
  a fresh `read_file`. It is for intentional complete rewrites, not recovery
  from an ambiguous patch. Files with mixed line endings fail closed.

`read_file` hashes the exact raw UTF-8 bytes, including any BOM and original
line endings, as `sha256:<lowercase hex>`. Its `lineEnding` metadata is `lf`,
`crlf`, `mixed`, or `none`.

Approval interruptions use the provider's stable tool-call identity scoped by
the June run. The pending interruption, serialized resume state, and run and
session waiting states commit atomically before the approval card is emitted.
Resolution is addressed by session, run, and interruption identity together,
so a provider-reused call id or stale card cannot authorize another run.

Patch and replacement preserve permissions, UTF-8 BOM, and uniform line-ending
style. They stage a new file in the target directory, sync it, reread and check
the target's original revision immediately before using the platform-aware
atomic replacement helper. Temporary files are removed after ordinary failures.
On an ambiguous Windows partial-replacement error, staged and backup recovery
files remain beside the target rather than risking deletion of the only intact
copy. Artifact bookkeeping runs only after a successful mutation and is
best-effort: a bookkeeping failure is logged and surfaced as non-fatal result
metadata rather than incorrectly reporting that an already-applied mutation
failed.

## Consequences

- Creation cannot accidentally destroy an existing file.
- Complete replacement is explicit, approval-gated, and rejects stale reads.
- Focused edits tolerate the common LF-anchor-to-CRLF-file mismatch without
  relaxing exact uniqueness.
- A non-cooperating process can still change the path after the final revision
  recheck and before atomic replacement. Eliminating that residual TOCTOU would
  require platform-specific locking or identity semantics that cooperating
  editors do not consistently honor.

## Alternatives considered

- **Keep overwrite behavior behind `write_file`.** Rejected because one tool
  cannot clearly communicate creation and destructive replacement intent.
- **Add a force option.** Rejected because it would bypass concurrency safety.
- **Normalize every file to LF.** Rejected because line endings are user-owned
  bytes and normalization creates noisy, surprising changes.
