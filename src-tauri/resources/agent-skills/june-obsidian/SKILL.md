---
name: june-obsidian
description: Works with the Obsidian vault currently selected in June. Use for Obsidian note tasks.
---

# June Obsidian vault

Use this skill for Obsidian note work. Before every distinct Obsidian task, call
`get_obsidian_vault` to discover the current vault.

- If `connected` is false, tell the user that no Obsidian vault is connected in
  June. Do not guess a default path.
- If `available` is false, tell the user that the connected vault is currently
  unavailable. Do not infer or reconstruct its absolute path.
- If a vault path is returned, it is current discovery only, not authorization.
  Stay within that vault. Do not infer write permission from receiving a path.
- Re-query when beginning another distinct task because the user may change or
  disconnect the vault while this session stays alive.

Use the returned absolute path with June's generic filesystem tools. Sandboxed
sessions may list, search, preview, and read the vault, but cannot change it.
Prefer filesystem tools over shell commands when practical. Vault updates
require an Unrestricted session and the normal write approval.

Follow Obsidian Markdown conventions: use YAML frontmatter only when the note
already uses it or the user asks for it, preserve valid frontmatter, and link
related notes with `[[Note Name]]` wikilinks.
