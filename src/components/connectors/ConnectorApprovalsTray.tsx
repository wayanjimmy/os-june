import { IconChecklist } from "central-icons/IconChecklist";
import { IconChevronDownSmall } from "central-icons/IconChevronDownSmall";
import { useCallback, useEffect, useRef, useState } from "react";
import { actionToolLabel, providerFromServer } from "../../lib/connectors";
import { useScrollFade } from "../../lib/use-scroll-fade";
import {
  CONNECTOR_APPROVALS_CHANGED_EVENT,
  type PendingConnectorApproval,
  connectorApprovalRespond,
  connectorApprovalsPending,
  connectorApprovalsRespondAll,
} from "../../lib/tauri";
import { ConnectorProviderIcon } from "./ConnectorProviderIcon";

/**
 * Always-mounted surface for connector action calls parked in the Rust proxy
 * under the approval trust mode. A routine (or chat) that tries to send, draft,
 * label, or change a calendar event pauses here until the user approves or
 * denies. Renders nothing when nothing is pending.
 *
 * Batched by design: a triage run can propose several drafts at once, so
 * "Approve all" answers the whole set in one tap. Every item shows the account
 * it touches and a redacted preview (redaction happens in Rust).
 */
export function ConnectorApprovalsTray() {
  const [pending, setPending] = useState<PendingConnectorApproval[]>([]);
  const [busy, setBusy] = useState(false);
  // Collapsed parks the tray as a single header line (the steer queue's
  // fold), so a batch the user isn't ready to answer stops covering the
  // composer corner. A fresh batch re-expands: new approvals demand eyes.
  const [collapsed, setCollapsed] = useState(false);
  const previousCount = useRef(0);
  const mounted = useRef(true);
  // The list clips to a max-height once a batch is long; the shared scroll
  // fade signals that more approvals are hidden below (spec/scroll-fade.md).
  const listRef = useRef<HTMLUListElement>(null);
  const fade = useScrollFade(listRef);

  const refresh = useCallback(async () => {
    try {
      const next = await connectorApprovalsPending();
      if (mounted.current) setPending(next);
    } catch {
      // The bridge may not be up yet; the change event will re-drive us.
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    void refresh();
    let unlisten: (() => void) | undefined;
    let aborted = false;
    import("@tauri-apps/api/event")
      .then(({ listen }) => listen(CONNECTOR_APPROVALS_CHANGED_EVENT, () => void refresh()))
      .then((cleanup) => {
        if (aborted) cleanup();
        else unlisten = cleanup;
      })
      .catch(() => {});
    return () => {
      mounted.current = false;
      aborted = true;
      unlisten?.();
    };
  }, [refresh]);

  const respondOne = useCallback(
    async (approvalId: string, approve: boolean) => {
      setBusy(true);
      try {
        await connectorApprovalRespond({ approvalId, approve });
      } finally {
        if (mounted.current) setBusy(false);
        void refresh();
      }
    },
    [refresh],
  );

  const respondAll = useCallback(
    async (approve: boolean) => {
      // Answer only the actions currently on screen. An action enqueued after
      // this snapshot has not been reviewed, so it must stay pending rather than
      // be swept into a bulk approve.
      const approvalIds = pending.map((item) => item.approvalId);
      if (approvalIds.length === 0) return;
      setBusy(true);
      try {
        await connectorApprovalsRespondAll({ approve, approvalIds });
      } finally {
        if (mounted.current) setBusy(false);
        void refresh();
      }
    },
    [pending, refresh],
  );

  // The list grows and shrinks as approvals arrive or are answered without a
  // scroll or resize, so nudge the shared fade to re-measure on each change.
  useEffect(() => {
    // Reading the length makes the change signal explicit to the hook linter.
    void pending.length;
    const id = requestAnimationFrame(fade.update);
    return () => cancelAnimationFrame(id);
  }, [pending.length, fade.update]);

  // Dev console driver (window.__connectorApprovals) that parks synthetic
  // approvals in the tray so its styling can be inspected without a live
  // routine proposing actions.
  useEffect(() => {
    if (!import.meta.env.DEV) return;
    let cancelled = false;
    let dispose: (() => void) | undefined;
    void import("../../lib/connector-approvals-demo").then(({ registerConnectorApprovalsDemo }) => {
      if (cancelled) return;
      ({ dispose } = registerConnectorApprovalsDemo({ setPending }));
    });
    return () => {
      cancelled = true;
      dispose?.();
    };
  }, []);

  // Re-expand when a batch arrives on an empty tray; a collapse mid-batch
  // sticks until the queue drains.
  useEffect(() => {
    if (pending.length > 0 && previousCount.current === 0) setCollapsed(false);
    previousCount.current = pending.length;
  }, [pending.length]);

  if (pending.length === 0) return null;

  return (
    <aside
      className="connector-approvals"
      aria-label="Connector approvals"
      // A status role fits a passive, self-updating queue better than a live
      // region alert.
      role="status"
    >
      <header className="connector-approvals-header">
        <button
          type="button"
          className="connector-approvals-trigger"
          aria-expanded={!collapsed}
          onClick={() => setCollapsed((current) => !current)}
        >
          <IconChecklist size={16} aria-hidden />
          Approvals needed
          {collapsed ? <span className="status-pill">{pending.length}</span> : null}
        </button>
        <span className="connector-approvals-header-actions">
          {!collapsed && pending.length > 1 ? (
            <span className="connector-approvals-bulk">
              <button
                type="button"
                className="btn btn-ghost connector-approvals-deny"
                disabled={busy}
                onClick={() => void respondAll(false)}
              >
                Deny all
              </button>
              <button
                type="button"
                className="btn btn-ghost"
                disabled={busy}
                onClick={() => void respondAll(true)}
              >
                Approve all
              </button>
            </span>
          ) : null}
          <button
            type="button"
            className="connector-approvals-chevron-button"
            aria-label={collapsed ? "Expand approvals" : "Collapse approvals"}
            aria-expanded={!collapsed}
            onClick={() => setCollapsed((current) => !current)}
          >
            <IconChevronDownSmall
              size={13}
              className="connector-approvals-chevron"
              data-expanded={!collapsed}
              aria-hidden
            />
          </button>
        </span>
      </header>
      {collapsed ? null : (
        <ul className="connector-approvals-list scroll-fade-mask" ref={listRef} {...fade.props}>
          {pending.map((item) => {
            const provider = providerFromServer(item.server);
            return (
              <li key={item.approvalId} className="connector-approvals-item">
                <span className="connector-approvals-mark" aria-hidden>
                  {provider ? (
                    <ConnectorProviderIcon provider={provider} size={14} />
                  ) : (
                    <IconChecklist size={14} aria-hidden />
                  )}
                </span>
                <div className="connector-approvals-info">
                  <p className="connector-approvals-summary">
                    {item.summary || actionToolLabel(item.tool)}
                  </p>
                  <p className="connector-approvals-meta">
                    {actionToolLabel(item.tool)} · {item.accountEmail}
                  </p>
                  {item.argsPreview ? (
                    <p className="connector-approvals-preview">{item.argsPreview}</p>
                  ) : null}
                </div>
                <div className="connector-approvals-actions">
                  <button
                    type="button"
                    className="btn btn-ghost connector-approvals-deny"
                    disabled={busy}
                    onClick={() => void respondOne(item.approvalId, false)}
                  >
                    Deny
                  </button>
                  <button
                    type="button"
                    className="connector-approvals-approve"
                    disabled={busy}
                    onClick={() => void respondOne(item.approvalId, true)}
                  >
                    Approve
                  </button>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </aside>
  );
}
