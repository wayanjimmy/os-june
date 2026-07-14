# Implementation plan: Obsidian plugin

**Owner:** CTO · **Date:** 2026-07-14 · **Status:** Proposed for product gate · **PRD:** [obsidian-prd.md](obsidian-prd.md)

## Technical objective

Expose a first-party local Obsidian plugin through app-owned MCP servers and a Rust vault broker. The plugin lets June search and read a selected Obsidian vault, then create or append Markdown notes only after the user approves an exact diff. The selected vault remains the source of truth; any index is derived and rebuildable.

Obsidian is not a connector. Do not add it to `ConnectorProvider`, `src-tauri/src/connectors/`, or connector account UI. A vault grant is local filesystem authority, not a third-party provider account.

## V1 scope

- macOS first.
- One active vault.
- Folder must contain `.obsidian/`.
- Markdown notes only.
- Read MCP server: `june_obsidian`.
- Action MCP server: `june_obsidian_actions`.
- Read tools: `search_notes`, `get_note`, `list_tags`, `get_backlinks`.
- Write tools: `create_note`, `append_note`.
- Write setting is off by default.
- Every write parks for approval with an exact escaped plain-text diff.
- No routines, autonomous grants, or unattended writes.
- No June API changes.

## Architecture

```text
Plugins or Settings UI
  -> local vault grant commands
  -> native folder picker
  -> vault health and write policy

Hermes
  -> june_obsidian MCP
       -> authenticated loopback route
       -> Rust Obsidian vault broker
       -> derived read index
       -> selected vault files

  -> june_obsidian_actions MCP
       -> authenticated loopback route
       -> Rust path and revision preflight
       -> pending write approval
       -> conflict recheck
       -> atomic filesystem write
```

Hermes and the Python MCP processes never receive the canonical root. The model sees only vault-relative paths returned by read tools. Rust is the only component allowed to cross from the sandboxed agent runtime into the user-selected vault.

## Rust module layout

Add `src-tauri/src/obsidian/`:

| Module | Responsibility |
| --- | --- |
| `mod.rs` | Public setup, lifecycle, shared state, events |
| `commands.rs` | Tauri commands for select, list, inspect, remove, rebuild, and write policy |
| `types.rs` | DTOs, health states, error codes, tool request and response structs |
| `broker.rs` | Grant resolution and read/write entry points used by loopback routes |
| `paths.rs` | Vault-relative path parsing and root confinement |
| `scanner.rs` | Bounded traversal and scan diagnostics |
| `parser.rs` | Markdown, frontmatter, tags, links, headings, and block references |
| `links.rs` | Wikilink and Markdown-link resolution |
| `index.rs` | Derived note, search, tag, alias, and backlink indexes |
| `watcher.rs` | File watching, event coalescing, overflow reconciliation |
| `approvals.rs` | Pending write plans and approve/decline commands |
| `writes.rs` | Revision checks, diff generation, temp writes, atomic replacement |

Register commands from `src-tauri/src/lib.rs`. Keep the first implementation specialized to Obsidian; do not prematurely extract a generic filesystem plugin framework.

## Data model

Add a migration such as `014_obsidian_vault_grants.sql`.

### `obsidian_vault_grants`

- `vault_id TEXT PRIMARY KEY` - random UUID, never derived from path.
- `display_name TEXT NOT NULL` - folder basename or user-facing label.
- `canonical_root TEXT NOT NULL` - Rust host only; never sent to Hermes.
- `root_identity TEXT NOT NULL` - platform file identity for root replacement detection.
- `write_enabled INTEGER NOT NULL DEFAULT 0`.
- `status TEXT NOT NULL` - last health state.
- `created_at TEXT NOT NULL`.
- `updated_at TEXT NOT NULL`.
- `last_checked_at TEXT`.
- `last_scan_started_at TEXT`.
- `last_scan_completed_at TEXT`.
- `last_successful_scan_at TEXT`.
- `index_version INTEGER NOT NULL DEFAULT 1`.
- `note_count INTEGER NOT NULL DEFAULT 0`.
- `tag_count INTEGER NOT NULL DEFAULT 0`.
- `unresolved_link_count INTEGER NOT NULL DEFAULT 0`.
- `ambiguous_link_count INTEGER NOT NULL DEFAULT 0`.
- `placeholder_file_count INTEGER NOT NULL DEFAULT 0`.
- `skipped_file_count INTEGER NOT NULL DEFAULT 0`.
- `last_error_code TEXT`.

V1 allows exactly one active row. Deleting the row revokes access, cancels pending approvals, drops the in-memory index, prunes MCP config, and restarts affected runtimes.

Do not persist note bodies, tags, aliases, backlinks, search terms, approval diffs, pending write plans, or loopback credentials in SQLite in v1.

## Broker invariants

Every operation must prove:

1. The plugin is enabled.
2. A current vault grant exists.
3. The opaque `vault_id` matches the current grant.
4. The root still exists and has the expected identity.
5. The requested path is a valid vault-relative path.
6. The resolved object stays inside the granted root.
7. No path component traverses a symlink, reparse point, or special file.
8. File type, file size, and encoding are supported.
9. Write operations have an approved immutable plan.
10. Source revision still matches immediately before commit.

## Path and filesystem policy

Accept only normalized vault-relative paths with `/` separators. Reject:

- absolute paths
- drive or UNC prefixes
- `.` and `..`
- NUL or control characters
- paths under `.obsidian`, `.git`, `.trash`, or June temp directories
- platform-reserved names
- non-regular files
- symlinks and reparse points
- unsupported extensions for V1 reads or writes

Use root-relative directory handles and no-follow semantics where available. Canonical string-prefix checks alone are not sufficient because a path component can be swapped between validation and use.

V1 skips and rejects all symlinks, even if their current target is inside the vault.

## Scanning and indexing

V1 index is derived in memory:

- exact vault-relative path to note record
- title or stem
- aliases
- normalized search terms and postings
- tags and counts
- outbound references
- resolved, unresolved, and ambiguous targets
- backlinks
- bounded content or snippet offsets
- revision metadata

Traversal is iterative and bounded. Scan only `.md` files. Exclude `.obsidian`, `.git`, `.trash`, and June temp files. Enforce file count, depth, source-byte, per-file, and wall-time caps. Build a new generation privately, then atomically swap it into service. Before the first complete generation, return `vault_indexing` rather than serving false-negative search results.

Search ranking:

1. Exact title.
2. Exact alias.
3. Path or title prefix.
4. Tag match.
5. Body term match.
6. Modified-time tie-break.

Do not add Tantivy or persistent SQLite FTS before benchmarks demonstrate the need.

## Watcher strategy

Treat filesystem events as hints, not truth. Debounce bursts, re-read metadata after it stabilizes, and reconcile after:

- watcher overflow
- watcher error
- app wake from sleep
- app focus
- drive unmount or remount
- periodic health check

Track self-generated writes by operation id and resulting revision so the watcher can update the index without treating the write as an external conflict.

## Cloud placeholder policy

Never silently hydrate cloud files. Detect unavailable iCloud or OneDrive placeholders where possible. Report `vault_file_not_local` and tell the user to make the file available locally. Never write to placeholder files.

## Parsing pipeline

For each supported note:

1. Read bounded bytes.
2. Reject invalid UTF-8 and NUL bytes.
3. Detect line endings.
4. Parse YAML frontmatter only at byte zero.
5. Parse Markdown structure with source spans.
6. Extract headings and explicit block ids.
7. Extract wikilinks and embeds outside code, comments, and escaped text.
8. Extract local Markdown links.
9. Extract inline and frontmatter tags.
10. Extract aliases.
11. Resolve links against the current index generation.
12. Update forward and reverse indexes.
13. Retain parser warnings instead of guessing through malformed or ambiguous data.

Use a real Markdown parser with source spans plus a bounded Obsidian syntax scanner. Do not implement the entire parser with regex.

## Obsidian format support

### Wikilinks

Support `[[Note]]`, `[[Folder/Note]]`, `[[Note.md]]`, `[[Note#Heading]]`, `[[Note#^block-id]]`, `[[Note|Label]]`, `![[Note]]`, and combinations.

Resolution order:

1. Parse embed marker, target, anchor, and label.
2. Test exact vault-relative path.
3. Find exact suffix matches for partial paths.
4. Find exact note-stem matches for pathless targets.
5. Prefer exact-case over normalized only when unique.
6. Accept only a unique result.
7. Otherwise record unresolved or ambiguous.

Do not reproduce undocumented nearest-folder tie-breaks. Duplicate basenames produce ambiguity, not a guessed backlink.

### Markdown links

Resolve relative Markdown links from the source note directory and root-style paths from the vault root. Percent-decode safely. Ignore external URL schemes. Never follow links outside the vault.

### Frontmatter

Recognize YAML only at the beginning of the file. Extract `tags`, `aliases`, and `cssclasses`. Keep unknown keys. Preserve raw bytes and source spans. Reject duplicate keys. Bound size, depth, and alias expansion. Do not load and dump frontmatter in v1. `create_note` accepts raw frontmatter only if it parses successfully.

### Tags

Extract inline tags outside code, comments, and URLs. Extract frontmatter tags from scalar and list forms. Store display spelling, Unicode-normalized comparison key, and hierarchy segments. Tag identity is case-insensitive. Parent matching is segment-aware.

### Embeds

Resolve embeds with the same algorithm as links and mark `embed=true`. V1 returns metadata only. Do not recursively inline embeds, read attachment bytes, or follow external paths.

### Case and Unicode

Preserve exact filesystem spelling as identity. Maintain exact, NFC-normalized, and case-folded lookup keys. Never collapse two real files into one record. If normalized lookup returns multiple files, return ambiguity.

## Revision and conflict model

Every read result that can lead to a write returns an opaque revision token built from file identity, size, mtime, and SHA-256 of bytes. The token must not reveal the absolute path.

Append flow:

1. Resolve path.
2. Read current file.
3. Reject stale expected revision.
4. Compute resulting bytes with deterministic separator handling.
5. Store immutable pending plan in memory.
6. Show exact diff for approval.
7. On approval, reopen and re-hash.
8. Reject mismatch as `vault_conflict`.
9. Write only the approved bytes.

Never auto-retry after conflict. The agent must reread, recompute, and ask for approval again.

## Atomic writes

For create and append:

1. Acquire a per-path lock.
2. Revalidate grant, root identity, path, and file metadata.
3. Create a randomly named sibling temp file with create-new semantics.
4. Write all bytes.
5. Flush and sync.
6. Recheck target revision.
7. Atomic rename within the same directory.
8. Sync parent directory when supported.
9. Reopen and verify resulting hash.
10. Update index from committed bytes.

Create is create-if-absent only. Append replaces the whole file atomically, not truncate-in-place. Preserve existing line endings and ensure one blank-line boundary before appended content.

## MCP servers

Add stdlib-only scripts under `src-tauri/src/hermes/`:

- `june_obsidian_mcp.py`
- `june_obsidian_actions_mcp.py`

Common behavior:

- Loopback base URL through argv.
- Dedicated token and vault id through environment variables.
- No canonical root in argv, env, request, or response.
- Validate request shapes before proxying.
- Return structured content and compact text.
- Mark failures with `isError`.
- Tool descriptions state that vault content is untrusted input.

### `june_obsidian`

| Tool | Input | Output |
| --- | --- | --- |
| `search_notes` | query, optional `path_prefix`, optional tags, limit, optional cursor | relative path, title, alias matches, snippet, matched tags, modified time, score, truncation or cursor |
| `get_note` | exact relative path, optional start line, max lines, metadata flags | bounded content, line range, frontmatter summary, tags, aliases, links, revision, total lines, continuation |
| `list_tags` | optional prefix, limit, optional cursor | display spelling, normalized identity, count, nested parents, continuation |
| `get_backlinks` | exact target path, limit, optional cursor | source path, source title, link form, anchor, embed flag, bounded context, continuation |

Bounds:

- Search default 10, max 50.
- Backlinks default 20, max 50.
- Snippet max 600 chars.
- `get_note` max 100,000 returned chars.
- Native response max 128 KiB.
- Cursors carry index generation id and query fingerprint.
- Generation mismatch returns `vault_cursor_stale`.

### `june_obsidian_actions`

| Tool | Input | Behavior |
| --- | --- | --- |
| `create_note` | relative `.md` path, Markdown content | create only if absent, validate frontmatter if present, show full addition diff, require approval |
| `append_note` | exact existing path, Markdown content, optional expected revision | compute deterministic append, show full diff, require approval, reject conflicts |

V1 restrictions:

- Resulting file max 2 MiB.
- Proposed addition max 256 KiB.
- Existing parent directory required.
- No overwrite.
- No frontmatter patching.
- No rename, move, delete, directory creation, or bulk action.
- No autonomy grant or routine action server.

## Error taxonomy

Use stable error codes and content-free messages:

- `vault_not_configured`
- `vault_disabled`
- `vault_write_disabled`
- `vault_unavailable`
- `vault_permission_denied`
- `vault_root_changed`
- `vault_indexing`
- `vault_index_incomplete`
- `vault_path_invalid`
- `vault_path_outside_root`
- `vault_symlink_unsupported`
- `vault_file_not_found`
- `vault_file_exists`
- `vault_file_not_local`
- `vault_file_too_large`
- `vault_file_not_utf8`
- `vault_frontmatter_invalid`
- `vault_link_ambiguous`
- `vault_cursor_stale`
- `vault_conflict`
- `vault_approval_declined`
- `vault_approval_timed_out`
- `vault_io_busy`
- `vault_internal_error`

Messages must not include canonical roots, file content, loopback tokens, or OS error strings containing sensitive paths.

## Frontend UX

Preferred implementation is a plugin detail page in the first-party Plugins foundation. If that foundation is unavailable, add a narrow static Settings surface for Obsidian rather than overloading Connectors.

State machine:

```text
Available -> Enabled -> Granted -> Healthy -> Write enabled -> Pending approval -> Runtime registered
```

Health states:

- No vault selected
- Indexing
- Healthy
- Missing
- Unreadable
- Permission denied
- Root changed
- Partial index
- Cloud files unavailable
- Watcher degraded
- Rebuilding
- Write conflict detected

Write approval UI must show:

- operation: create or append
- vault display name
- exact relative path
- whether the note exists
- complete escaped plain-text unified diff
- added line and byte counts
- parser warnings
- Approve and Decline actions

Do not offer bulk approval or "always allow". Approval expires after a bounded window. Removing the grant or disabling writes cancels pending approvals.

Frontend events:

- `june://obsidian-vault-changed`
- `june://obsidian-index-changed`
- `june://obsidian-approvals-changed`

Add typed wrappers in `src/lib/tauri.ts` for status, select, confirm, remove grant, set write mode, rebuild index, pending writes, approve or decline one write, and apply runtime.

## Security and privacy

Assets to protect:

- files outside the selected root
- unsupported files inside the root
- note integrity and concurrent user edits
- canonical root path
- note content in logs and telemetry
- loopback credentials
- June plugin and write-policy state

Trust boundaries:

- Vault files are untrusted input.
- Hermes and MCP processes are untrusted for policy enforcement.
- The frontend is presentation, not authorization.
- Rust broker is the enforcement point.
- Local OS and user account are trusted.

Privacy copy must say: vault reads and writes stay on-device, but content used in an agent run may be included in inference prompts and usually transits June API unless the user selects a local model. Do not claim that content never leaves the device unless local inference is active.

Telemetry must exclude canonical paths, note titles, tags, queries, bodies, and diffs. Content-free local counters may record scan duration buckets, file-count buckets, errors, approvals, denials, and conflicts only if existing opt-in P3A policy permits them.

## Revocation

Removing vault access must:

1. Mark the grant unavailable immediately.
2. Reject new broker requests.
3. Cancel pending approvals.
4. Stop the watcher.
5. Drop the in-memory index.
6. Delete the SQLite grant and health snapshot.
7. Prune MCP config and restart affected runtimes.
8. Verify stale MCP calls return `vault_not_configured`.
9. Never modify or delete vault files.

## Performance caps

Start with explicit constants and tune during the spike:

- 50,000 Markdown files.
- Traversal depth 64.
- 2 MiB per indexed note.
- 512 MiB total source bytes per full scan.
- 256 KiB frontmatter cap.
- 256 KiB proposed write cap.
- 2 MiB resulting-note cap.
- 50 maximum search or backlink results.
- 128 KiB maximum MCP response.
- Bounded watcher queue, overflow forces reconciliation.

Target benchmarks on a documented reference Mac:

- Settings health snapshot under 200 ms.
- Search after indexing p95 under 300 ms for 10,000 notes.
- Exact `get_note` p95 under 100 ms.
- Initial index of 10,000 median-sized notes under 10 seconds.
- Incremental single-note reparse under 100 ms.
- Index memory under 200 MiB for the benchmark vault.

## Testing strategy

Rust unit tests:

- absolute paths, parent traversal, mixed separators, platform prefixes
- symlinked parent, symlinked file, symlinked root
- root replaced after grant
- case-only names and Unicode normalization collisions
- reserved names and special files
- `.obsidian`, `.git`, `.trash`, and June temp exclusions
- malformed and valid frontmatter
- duplicate YAML keys
- scalar and list aliases and tags
- inline tags around code, comments, URLs, and punctuation
- wikilinks with paths, labels, headings, blocks, and embeds
- escaped brackets and pipes
- Markdown local links
- duplicate headings and block ids
- invalid UTF-8 and oversized notes
- unique and ambiguous basename resolution
- append/create conflicts and approval timeout
- post-write hash mismatch
- stale request after grant removal

Property and fuzz tests:

- frontmatter delimiters and YAML structures
- wikilink scanner
- Markdown and link nesting
- arbitrary relative paths
- malformed UTF-8 byte streams
- resolved paths never escape the root
- index rebuild from identical bytes produces identical graph state
- approved plan yields exactly its approved resulting hash or no write
- ambiguous candidates are never silently reduced to one
- parser or index failures never mutate source files

Integration tests:

- temporary vault with live watcher
- initial scan and incremental changes
- event overflow followed by reconciliation
- MCP script against fake authenticated loopback broker
- token isolation across model, connector, recorder, and Obsidian routes
- read server cannot call action routes
- grant removal invalidates already running MCP server
- runtime config adds and prunes both servers correctly
- pinned Hermes discovers and calls all tool schemas

Frontend tests:

- all health states
- picker success, cancellation, and failure
- privacy disclosure and support matrix
- write setting defaults off
- approval diff escaping
- conflict, denial, timeout, and success notices
- event listener cleanup
- removal confirmation says files are not deleted
- sentence-case, central-icon, token, control-size, and scroll-fade compliance

## Delivery phases

### Phase 0: Product and threat-model gate

**Estimate:** 1-2 product days plus 2-3 engineering spike days.

Tasks:

- Score Obsidian against the portfolio rubric.
- Validate two graph-aware core jobs.
- Decide macOS, one-vault, `.obsidian` marker, writes, and routine scope.
- Create support matrix and privacy copy.
- Spike no-follow path access and iCloud placeholder detection.
- Benchmark representative small, medium, and large vaults.
- Accept ADR and threat model.

Exit: separate plugin approved, v1 scope frozen, path-safety design demonstrated, no unresolved blocker around persistent folder access.

### Phase 1: Local plugin and grant foundation

**Estimate:** 3-5 engineering days.

Tasks:

- Add static first-party manifest or integrate with Plugins foundation.
- Add plugin enabled/write state.
- Add vault-grant migration and repository methods.
- Add `obsidian` Rust module and managed broker state.
- Add selection, list, health, remove, and rebuild commands.
- Add Settings tile/detail and native picker.
- Add runtime apply and stale-config pruning.
- Add dedicated loopback credential and route family.

Exit: a vault can be selected, persisted, inspected, removed, and revoked. No MCP tool can access a path yet. Revocation tests pass.

### Phase 2: Read-only vertical slice

**Estimate:** 2-3 weeks, including Phase 1 if done by the same engineer.

Tasks:

- Bounded traversal and initial index.
- Markdown, frontmatter, link, and tag parser.
- Exact and normalized path indexes.
- Wikilink resolution and explicit ambiguity.
- Backlink and tag indexes.
- Watcher, event coalescing, and reconciliation.
- Cloud-placeholder health handling.
- `june_obsidian_mcp.py`.
- `search_notes`, `get_note`, `list_tags`, `get_backlinks`.
- Conditional MCP registration and soul guidance.
- Result bounds, cursors, errors, and metrics.
- Full fixture and integration coverage.

Exit: representative 10,000-note vault meets agreed search and memory targets, external changes appear without restart, no absolute path reaches Hermes, ambiguous links are never guessed, read-only rc dogfood can begin.

### Phase 3: Approved safe writes

**Estimate:** 1-2 weeks.

Tasks:

- Write setting, off by default.
- `june_obsidian_actions_mcp.py`.
- `create_note` and `append_note`.
- Revision tokens and immutable pending plans.
- Exact diff generation.
- Approval UI and Tauri event flow.
- Per-path serialization.
- Same-directory temp writes, sync, rename, and verification.
- Conflict and revocation behavior.
- Cleanup of abandoned temporary files.
- Concurrency and crash-path tests.

Exit: every mutation is approved against exact bytes, create never overwrites, append never commits after a detected conflict, failure before replacement leaves original intact, no routine or autonomous path can call write tools.

### Phase 4: Skills, hardening, and rc

**Estimate:** about 1 week.

Tasks:

- Meeting-to-decision skill.
- Project-update skill.
- Research-brief skill.
- Dogfood corpus and workflow QA.
- Performance tuning.
- Support matrix and troubleshooting documentation.
- Content-free telemetry review.
- Kill switch and rollback runbook.
- Signed macOS qualification.
- RC rollout.

Exit: skills use exact paths and revisions correctly, approval completion meets the portfolio quality target, privacy and support claims match actual behavior, no unresolved high-severity filesystem issue.

## Risks and mitigations

| Risk | Mitigation |
| --- | --- |
| Product overlap with Documents | Require graph-aware jobs and `.obsidian` semantics at the product gate |
| Missing Plugins foundation | Make it a prerequisite or implement a static first-party manifest only |
| Symlink or traversal escape | Root-relative handles, no-follow traversal, reject all links and reparse points in v1 |
| Canonical-root replacement | Persist and recheck root file identity |
| Concurrent Obsidian or sync writes | Revision tokens, approval baseline, immediate recheck, atomic replacement, no auto-retry |
| Cloud placeholder blocks or hydrates | Platform detection, skip unavailable files, never write placeholders |
| Large vault performance | Hard caps, generation swap, incremental watcher, benchmarks, persistent FTS only if needed |
| Watcher event loss | Treat events as hints, overflow detection, periodic reconciliation |
| Undocumented Obsidian resolution behavior | Explicit ambiguity and published compatibility contract |
| Unicode or case collisions | Exact identity plus normalized indexes, never collapse candidates |
| Prompt injection in notes | Untrusted-input guidance, Rust policy, exact diff, no autonomy |
| Sensitive data in logs | Stable codes only; no roots, content, queries, or diffs in logs or telemetry |
| Config merge preserves stale MCP entries | Explicitly prune all dynamic `june_obsidian*` entries before merge |
| Action server leaks to routines | Exclude from routine toolsets in v1; no autonomy server or grant |
| Temp files appear in Obsidian | Hidden reserved prefix, short lifetime, startup cleanup, watcher suppression |
| Trademark or implied endorsement | Naming and listing review |

## Open decisions

1. Separate plugin or Documents feature? Recommended: separate if graph jobs validate.
2. Portfolio priority: which ranked workstream, if any, does Obsidian displace?
3. Plugin foundation dependency: use JUN-275 or ship static first-party state?
4. Target platform: macOS-only v1?
5. Number of vaults: exactly one active vault in v1?
6. Require `.obsidian/` marker? Recommended: yes.
7. Write default: separately opt-in and approval-only? Recommended: yes.
8. Routines and autonomy excluded? Recommended: yes.
9. Symlink behavior: skip and reject all symlinks/reparse points? Recommended: yes.
10. Hidden folders: exclude `.obsidian`, `.git`, `.trash`, and June temp files?
11. Cloud placeholders: never hydrate automatically?
12. Index persistence: in-memory v1?
13. Attachments: parse references but do not read bytes in v1?
14. Malformed frontmatter: allow reads with warnings, reject creation?
15. Create parent directories: no in v1?
16. Append separator: preserve file line endings and ensure one blank-line boundary?
17. Maximum supported note and vault size: accept caps during Phase 0?
18. Persistent folder access: are security-scoped bookmarks needed for signed macOS builds?
19. Atomic replacement metadata: is preserving POSIX mode sufficient?
20. Naming and trademark: Obsidian, Obsidian vault, or Markdown vault with Obsidian support?
