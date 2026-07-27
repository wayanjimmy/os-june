---
status: accepted
date: 2026-07-27
---

# June-owned bundled agent skills are read-only app resources

## Context

ADR-0011 introduced read-only app resources for selected skills because June's
then-pinned Hermes runtime did not contain them. ADR-0034 used that mechanism
for the distinct `june-obsidian` skill, which carries the task-time discovery,
safety, and Markdown behavior for the Obsidian plugin.

ADR-0038 replaced Hermes with June's OpenAI Agents SDK harness and made June's
SQLite database and Rust host authoritative for skills. The cutover removed the
Hermes resource tree, including `june-obsidian`, but retained only a shortened
version of its instructions in the native `get_obsidian_vault` descriptor. The
new skill catalog searches June-managed app data and user-global
`~/.agents/skills`; it has no source for read-only skills shipped by June.

Copying first-party skills into managed app data would make release-owned
content look user-editable and require synchronization or overwrite rules.
Encoding every workflow only in tool descriptions would make skills disappear
from the catalog and overload tool metadata with instructions that are relevant
only when a workflow is selected.

## Decision

June may ship first-party agent skills under
`src-tauri/resources/agent-skills/` as read-only Tauri resources. The bundle
maps that directory to `native/agent-skills`, and development builds read the
same source directory directly.

Skill lookup precedence is:

1. June-managed skills in the app data directory.
2. User-global skills under `~/.agents/skills`.
3. June-bundled read-only skills.

The first matching stable skill id wins. This preserves user and managed
overrides while ensuring a release-owned fallback is available without copying
files into mutable state. Bundled skills appear in the catalog with source
`bundled`, are enabled by default unless the user has stored a disabled skill
setting for that id, and cannot be edited through June.

June restores `june-obsidian` through this mechanism. It calls the Rust-owned
`get_obsidian_vault` tool rather than the retired `june_obsidian` MCP server.
The native tool remains available independently and keeps a concise discovery
description so correct behavior does not depend entirely on loading the skill.

This ADR supersedes ADR-0011's Hermes-specific resource ownership with a
June-owned equivalent. It supersedes ADR-0034's Hermes and MCP transport details
for the skill and discovery tool while retaining ADR-0034's behavior, freshness,
privacy, and authorization decisions. ADR-0038's June-owned runtime boundary
remains unchanged.

## Consequences

- First-party workflow instructions remain visible through the same skill
  catalog and `list_skills` / `load_skill` tools as installed skills.
- App releases update bundled skills without mutating managed or user-global
  skill files.
- Managed and user-global skills may intentionally shadow a bundled skill with
  the same stable id.
- The desktop bundle gains only the selected skill files; Hermes, its skill
  loader, MCP loopback, and profile synchronization do not return.
- Tool descriptors still carry the minimum contract needed to invoke a native
  tool safely when its related skill has not been loaded.

## Alternatives considered

- **Keep all Obsidian instructions in the tool descriptor.** Rejected because
  ADR-0034 defines Obsidian as a discoverable skill and workflow guidance is
  broader than one tool invocation.
- **Copy bundled skills into managed app data.** Rejected because it blurs
  release and user ownership and requires conflict and upgrade semantics.
- **Restore the Hermes resource directory.** Rejected because Hermes is retired
  and the June-owned harness has its own skill catalog.
