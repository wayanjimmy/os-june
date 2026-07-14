import { errorCode } from "./errors";
import type { ObsidianVaultHealth } from "./tauri";

export type ObsidianHealthMeta = {
  label: string;
  tone: "ok" | "attention" | "neutral";
  blurb: string;
};

export const OBSIDIAN_HEALTH_META: Readonly<Record<ObsidianVaultHealth, ObsidianHealthMeta>> =
  Object.freeze({
    no_vault_selected: {
      label: "No vault selected",
      tone: "neutral",
      blurb: "Select a local Obsidian vault before June can use vault tools.",
    },
    indexing: {
      label: "Indexing",
      tone: "neutral",
      blurb: "June is preparing a local, rebuildable vault index.",
    },
    healthy: {
      label: "Healthy",
      tone: "ok",
      blurb: "The selected vault is available on this Mac.",
    },
    missing: {
      label: "Missing",
      tone: "attention",
      blurb: "The selected vault folder is no longer available.",
    },
    unreadable: {
      label: "Unreadable",
      tone: "attention",
      blurb: "June cannot read the selected vault folder.",
    },
    permission_denied: {
      label: "Permission denied",
      tone: "attention",
      blurb: "macOS denied access to the selected vault folder.",
    },
    root_changed: {
      label: "Root changed",
      tone: "attention",
      blurb: "The selected folder no longer matches the granted vault.",
    },
    partial_index: {
      label: "Partial index",
      tone: "attention",
      blurb: "Some notes were skipped because limits or parsing warnings were hit.",
    },
    cloud_files_unavailable: {
      label: "Cloud files unavailable",
      tone: "attention",
      blurb: "Some vault files are not downloaded locally.",
    },
    watcher_degraded: {
      label: "Watcher degraded",
      tone: "attention",
      blurb: "June will reconcile the vault with slower background checks.",
    },
    rebuilding: {
      label: "Rebuilding",
      tone: "neutral",
      blurb: "June is rebuilding the local vault index.",
    },
    write_conflict_detected: {
      label: "Write conflict detected",
      tone: "attention",
      blurb: "A note changed while June was preparing a write.",
    },
  });

export function obsidianHealthMeta(status: ObsidianVaultHealth): ObsidianHealthMeta {
  return OBSIDIAN_HEALTH_META[status];
}

export function isMissingObsidianMarkerError(err: unknown): boolean {
  return errorCode(err) === "vault_marker_missing";
}
