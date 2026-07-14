# Obsidian local vault plugin via Rust broker

## Status

proposed - records the product and architecture gate for the proposed
Obsidian plugin. It does not authorize implementation before the product gate
in [obsidian-prd.md](../plugins/obsidian-prd.md) is accepted.

## Context

June is evaluating an Obsidian plugin. An Obsidian vault is a user-selected
local folder of Markdown files, often with `.obsidian/` metadata, wikilinks,
frontmatter, tags, aliases, backlinks, embeds, and optional Canvas files. This
is materially different from the existing Google connector work: there is no
third-party account, OAuth grant, provider API, refresh token, provider scope,
or Keychain token custody.

The tempting shortcut is to add Obsidian to the connector framework or to let
Hermes operate directly on a selected folder. Both would break important
boundaries:

1. `ConnectorProvider` represents authorized third-party accounts and their
   token lifecycle. Modeling a local folder as a connector would make future
   code and UI ask the wrong questions: account, scope, reconnect, provider
   revocation, and provider data path.
2. Sandboxed Hermes child processes are not trusted with broad filesystem
   authority. A vault write intentionally crosses the sandbox boundary into a
   user-owned folder, so policy must live in the Rust host.
3. Vault content is untrusted input. Notes may contain instructions that should
   be treated as data, not as authority to bypass path validation, approval, or
   write policy.
4. Obsidian graph behavior is not generic document editing. If the product value
   is only Markdown import or export, it belongs in Documents rather than a
   separate plugin.

## Decision

Obsidian is a separate first-party local plugin only if the product gate proves
that graph-aware vault workflows deserve their own surface. The plugin must not
be implemented as a connector.

If approved, the V1 shape is:

- macOS first.
- One active vault selected by the user.
- Folder must contain `.obsidian/`.
- Markdown notes only.
- No note content persisted in SQLite.
- A local vault grant persisted in SQLite with an opaque `vault_id`, display
  name, canonical root for Rust only, root identity, write policy, and health
  metadata.
- App-owned read MCP server `june_obsidian` for search, note reads, tags, and
  backlinks.
- App-owned action MCP server `june_obsidian_actions` for create and append.
- Python MCP servers receive a loopback base URL, a dedicated bearer token, and
  the opaque vault id. They never receive the canonical root.
- Rust vault broker owns root validation, path confinement, scanning, parsing,
  indexing, approval, conflict detection, and filesystem writes.
- Every V1 mutation requires June's own approval UI with an exact escaped
  plain-text diff. There are no routines, autonomous grants, or unattended
  writes in V1.
- Revocation immediately rejects new broker calls, cancels pending approvals,
  drops the in-memory index, deletes local grant state, prunes `june_obsidian*`
  MCP config, restarts affected runtimes, and never modifies vault files.

## Consequences

- The connector taxonomy stays clean: connectors are for third-party accounts;
  a local vault grant is filesystem authority for a selected local root.
- The implementation needs a new `src-tauri/src/obsidian/` module rather than
  new connector provider variants.
- The Hermes config merge must explicitly prune stale `june_obsidian*` entries,
  following the connector precedent, so removing a vault cannot leave stale MCP
  tools exposed.
- The Rust broker becomes the enforcement point. The frontend presents state and
  approvals, while Hermes and MCP scripts are untrusted for policy decisions.
- Privacy copy must distinguish local vault reads and writes from inference:
  filesystem operations stay on-device, but content used in an agent run may
  transit June API unless a local model is selected.
- A future design that adds routines, autonomous writes, multiple vaults,
  attachments, rename/move/delete, Canvas writes, or cloud-held vault access
  requires a new or superseding decision record.

## Alternatives considered

### Add Obsidian to Connectors

Rejected. It would reuse account, OAuth, scopes, reconnect, and token-custody
concepts that do not exist for a local folder, and would blur the product
language in `CONTEXT.md`.

### Let Hermes access the vault directly

Rejected. It widens sandboxed runtime authority and moves path policy and write
approval into an untrusted process. It also makes revocation and stale runtime
state harder to verify.

### Fold into Documents

Deferred to the product gate. If dogfood shows users only want generic Markdown
file creation or export, Documents is the right home. A separate Obsidian plugin
is justified only by graph-aware retrieval and graph-aware filing.
