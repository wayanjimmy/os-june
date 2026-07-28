---
status: accepted
date: 2026-07-27
---

# Sandboxed host file tools separate read and write policy

## Context

ADR-0038 placed machine access in Rust-owned host tools and stated that
Windows file tools enforce workspace roots. The initial implementation applied
that rule to reads and writes alike. This contradicted the Runtime mode promise
that Sandboxed sessions can read user files but can only change their own
workspace. It also made ADR-0034's Obsidian workflow unusable after vault
discovery because the selected vault normally lives outside the session
workspace.

Treating a discovered vault as an implicit read grant would not repair the
general Runtime mode contract. It would introduce a separate authorization
model without consent, lifetime, attachment, revocation, or canonical-root
semantics.

## Decision

Rust host file tools classify paths by operation:

- Read operations may access any existing path readable by the June process.
  Relative paths still resolve from the canonical session workspace.
- In Sandboxed mode, every mutation must resolve inside the canonical session
  workspace. This includes patching an existing file and generated artifact
  destinations.
- The Rust host derives the Sandboxed workspace from app data and ignores a
  caller-supplied workspace path, so the renderer cannot move the write boundary.
- In Unrestricted mode, mutations may target external paths through the normal
  approval flow.
- Tools with a source and destination validate each independently. An external
  source does not authorize an external destination.
- New write paths resolve through a canonical existing ancestor and reject
  unresolved traversal components. Existing write targets are canonicalized so
  symlinks cannot escape the workspace check.

Obsidian discovery remains current state, not authorization. The generic read
policy is what permits vault reads. Disconnecting a vault removes future
discovery but does not revoke operating-system access or a path already
disclosed to a live run.

The shell policy remains separate and unchanged: macOS uses the Seatbelt write
jail, while Windows Sandboxed sessions cannot run shell commands.

## Consequences

- Sandboxed sessions can list, search, preview, and read a selected Obsidian
  vault while writes and patches remain workspace-confined.
- External image-edit and import sources follow the same read policy.
- macOS TCC and filesystem permissions may deny protected reads. Windows ACLs
  generally permit the desktop process to read files owned by the same user,
  so Sandboxed mode is a mutation boundary rather than a per-file privacy
  boundary.
- Canonicalization closes ordinary symlink and traversal escapes. Replacement
  by another same-user process between validation and I/O remains a residual
  time-of-check/time-of-use risk.

This decision narrows ADR-0038's Windows workspace-root statement: workspace
confinement applies to Sandboxed host-tool mutations, not host-tool reads.

## Alternatives considered

- **Authorize only the selected Obsidian vault.** Rejected because discovery is
  explicitly not authorization and other user-file workflows require reads.
- **Add explicit per-run read grants now.** Deferred as a separate privacy
  design requiring a consent surface and durable grant semantics.
- **Require Unrestricted mode for vault reads.** Rejected because it contradicts
  the existing Runtime mode contract and unnecessarily enables mutations.
