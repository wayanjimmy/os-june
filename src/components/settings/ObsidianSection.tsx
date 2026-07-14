import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { obsidianHealthMeta, isMissingObsidianMarkerError } from "../../lib/obsidian";
import { messageFromError } from "../../lib/errors";
import {
  OBSIDIAN_VAULT_CHANGED_EVENT,
  obsidianVaultConfirm,
  obsidianVaultRemove,
  obsidianVaultSetWriteMode,
  obsidianVaultStatus,
  type ObsidianVaultGrant,
} from "../../lib/tauri";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { InlineNotice } from "../ui/InlineNotice";
import { Switch } from "../ui/Switch";
import { toast } from "../ui/Toaster";
import { SettingsPageHeader } from "./AppSettings";

function formatCount(count: number, singular: string): string {
  const label = count === 1 ? singular : `${singular}s`;
  return `${count.toLocaleString()} ${label}`;
}

function vaultSubtitle(grant: ObsidianVaultGrant): string {
  const stats = [
    formatCount(grant.noteCount, "note"),
    formatCount(grant.tagCount, "tag"),
  ];
  return `${grant.displayPath} · ${stats.join(" · ")}`;
}

/** Static first-party plugin surface for the Obsidian foundation slice. It is
 * intentionally separate from Connectors: selecting a vault grants local
 * filesystem authority, not a provider account or OAuth scope. */
export function ObsidianSection() {
  const [grant, setGrant] = useState<ObsidianVaultGrant | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [removeOpen, setRemoveOpen] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const status = await obsidianVaultStatus();
      setGrant(status.grant ?? null);
      setLoadError(null);
    } catch (err) {
      setLoadError(messageFromError(err));
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void refresh();
    void listen(OBSIDIAN_VAULT_CHANGED_EVENT, () => void refresh()).then((cleanup) => {
      if (cancelled) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [refresh]);

  async function selectVault() {
    if (busy) return;
    setBusy(true);
    try {
      const selected = await openDialog({ directory: true, multiple: false });
      if (typeof selected !== "string") return;
      const next = await obsidianVaultConfirm(selected);
      setGrant(next);
      toast.success("Vault selected");
    } catch (err) {
      const message = isMissingObsidianMarkerError(err)
        ? "Choose a folder that contains an .obsidian directory."
        : messageFromError(err);
      toast.error(message);
    } finally {
      setBusy(false);
    }
  }

  async function setWriteEnabled(writeEnabled: boolean) {
    if (!grant || busy) return;
    setBusy(true);
    try {
      const next = await obsidianVaultSetWriteMode({ vaultId: grant.vaultId, writeEnabled });
      setGrant(next);
      toast.success(writeEnabled ? "Vault writes enabled" : "Vault writes disabled");
    } catch (err) {
      toast.error(messageFromError(err));
    } finally {
      setBusy(false);
    }
  }

  async function confirmRemove() {
    if (!grant || busy) return;
    setBusy(true);
    try {
      await obsidianVaultRemove(grant.vaultId);
      setGrant(null);
      setRemoveOpen(false);
      toast.success("Vault access removed");
    } catch (err) {
      toast.error(messageFromError(err));
    } finally {
      setBusy(false);
    }
  }

  const health = grant ? obsidianHealthMeta(grant.status) : null;

  return (
    <section className="settings-group" aria-labelledby="obsidian-heading">
      <SettingsPageHeader
        id="obsidian-heading"
        title="Obsidian"
        blurb="Select one local vault for June to read. Vault file access stays on this Mac; note content used in agent runs may still be sent for model inference."
      />

      {loadError ? (
        <InlineNotice tone="warning" body={loadError} aria-label="Obsidian load error" />
      ) : null}

      <div className="settings-card">
        <div className="settings-rows">
          <div className="settings-row settings-row-meta">
            <div className="settings-row-info">
              <h3 className="settings-row-title">Vault</h3>
              <p className="settings-row-description">
                {grant ? vaultSubtitle(grant) : "No vault selected. Choose an Obsidian folder to grant local access."}
              </p>
            </div>
            <div className="settings-row-control">
              <button type="button" className="btn btn-secondary" onClick={selectVault} disabled={busy}>
                {grant ? "Change vault" : "Select vault"}
              </button>
            </div>
          </div>

          {grant && health ? (
            <>
              <div className="settings-row settings-row-meta">
                <div className="settings-row-info">
                  <h3 className="settings-row-title">Health</h3>
                  <p className="settings-row-description">{health.blurb}</p>
                </div>
                <div className="settings-row-control">
                  <span
                    className="status-pill"
                    data-tone={health.tone === "ok" ? "ok" : health.tone === "attention" ? "warning" : undefined}
                    title={health.blurb}
                  >
                    {health.label}
                  </span>
                </div>
              </div>

              <div className="settings-row settings-row-meta">
                <div className="settings-row-info">
                  <h3 className="settings-row-title">Write access</h3>
                  <p className="settings-row-description">
                    Off by default. Create and append operations will still require an exact diff approval.
                  </p>
                </div>
                <div className="settings-row-control">
                  <Switch
                    checked={grant.writeEnabled}
                    onCheckedChange={setWriteEnabled}
                    disabled={busy}
                    aria-label="Toggle Obsidian write access"
                  />
                </div>
              </div>

              <div className="settings-row settings-row-meta">
                <div className="settings-row-info">
                  <h3 className="settings-row-title">Remove vault access</h3>
                  <p className="settings-row-description">
                    Removes June's local grant and leaves your vault files untouched.
                  </p>
                </div>
                <div className="settings-row-control">
                  <button
                    type="button"
                    className="btn btn-secondary"
                    onClick={() => setRemoveOpen(true)}
                    disabled={busy}
                  >
                    Remove access
                  </button>
                </div>
              </div>
            </>
          ) : null}
        </div>
      </div>

      <ConfirmDialog
        open={removeOpen}
        title="Remove vault access?"
        description="June will forget this local vault grant. Your Obsidian files will not be changed or deleted."
        confirmLabel="Remove access"
        confirmBusyLabel="Removing..."
        cancelLabel="Cancel"
        destructive
        onConfirm={confirmRemove}
        onClose={() => setRemoveOpen(false)}
      />
    </section>
  );
}
