import { IconArrowRotateClockwise } from "central-icons/IconArrowRotateClockwise";
import { useState } from "react";
import {
  osAccountsCancelLogin,
  osAccountsLogin,
  osAccountsLogout,
  osAccountsTopUp,
} from "../../lib/tauri";
import type { AccountStatus } from "../../lib/tauri";

type Props = {
  account: AccountStatus;
  loading: boolean;
  onAccountChanged: (next: AccountStatus) => void;
  onRefresh: () => Promise<AccountStatus | undefined>;
};

export function AccountSettings({
  account,
  loading,
  onAccountChanged,
  onRefresh,
}: Props) {
  const [refreshing, setRefreshing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<string>();

  async function handleSignIn() {
    setBusy(true);
    setStatus("Opening your browser to sign in…");
    try {
      const next = await osAccountsLogin();
      onAccountChanged(next);
      setStatus(
        next.signedIn ? `Signed in as ${displayName(next)}.` : undefined,
      );
    } catch (error) {
      setStatus(messageFromError(error));
    } finally {
      setBusy(false);
    }
  }

  async function handleCancel() {
    // Aborts the waiting os_accounts_login; that promise then rejects with
    // "login_canceled", and handleSignIn's catch/finally resets status + busy.
    try {
      await osAccountsCancelLogin();
    } catch (error) {
      setStatus(messageFromError(error));
    }
  }

  async function handleSignOut() {
    setBusy(true);
    try {
      await osAccountsLogout();
      onAccountChanged({ signedIn: false, configured: account.configured });
      setStatus("Signed out.");
    } catch (error) {
      setStatus(messageFromError(error));
    } finally {
      setBusy(false);
    }
  }

  async function handleTopUp() {
    try {
      await osAccountsTopUp();
      setStatus("Opened OS Accounts. Your balance updates after checkout.");
    } catch (error) {
      setStatus(messageFromError(error));
    }
  }

  async function handleRefresh() {
    setRefreshing(true);
    try {
      await onRefresh();
    } finally {
      setRefreshing(false);
    }
  }

  return (
    <div className="settings-page">
      <header className="settings-header">
        <h1 className="settings-title">Account</h1>
        <p className="settings-description">
          Sign in with Open Software to use your shared identity and credit
          balance across the network.
        </p>
        {status ? <p className="settings-status">{status}</p> : null}
      </header>

      <section className="settings-group" aria-labelledby="identity-heading">
        <h2 id="identity-heading" className="settings-group-heading">
          Identity
        </h2>
        <div className="settings-card">
          <div className="settings-rows">
            <div className="settings-row">
              <div className="settings-row-info">
                <h3 className="settings-row-title">
                  {loading
                    ? "Checking sign-in…"
                    : account.signedIn
                      ? displayName(account)
                      : "Not signed in"}
                </h3>
                <p className="settings-row-description">
                  {account.signedIn
                    ? (account.user?.email ??
                      `@${account.user?.handle ?? "account"}`)
                    : "Your login and credits are managed by Open Software."}
                </p>
              </div>
              <div className="settings-row-control">
                {account.signedIn ? (
                  <button
                    type="button"
                    className="btn btn-secondary"
                    disabled={busy}
                    onClick={() => void handleSignOut()}
                  >
                    Sign out
                  </button>
                ) : busy ? (
                  <button
                    type="button"
                    className="btn btn-secondary"
                    onClick={() => void handleCancel()}
                  >
                    Cancel
                  </button>
                ) : (
                  <button
                    type="button"
                    className="btn btn-secondary"
                    disabled={loading}
                    onClick={() => void handleSignIn()}
                  >
                    Sign in with Open Software
                  </button>
                )}
              </div>
            </div>
          </div>
        </div>
      </section>

      {account.signedIn ? (
        <section className="settings-group" aria-labelledby="credits-heading">
          <h2 id="credits-heading" className="settings-group-heading">
            Credits
          </h2>
          <div className="settings-card">
            <div className="settings-rows">
              <div className="settings-row">
                <div className="settings-row-info">
                  <h3 className="settings-row-title">
                    {formatCredits(account.balance?.credits)} credits
                  </h3>
                  <p className="settings-row-description">
                    {formatUsd(account.balance?.usdMillis)} available. Credits
                    are added by Open Software after checkout.
                  </p>
                </div>
                <div className="settings-row-control">
                  <button
                    type="button"
                    className="btn btn-ghost"
                    aria-label="Refresh balance"
                    title="Refresh balance"
                    disabled={refreshing}
                    onClick={() => void handleRefresh()}
                  >
                    <IconArrowRotateClockwise
                      size={14}
                      data-spinning={refreshing ? "true" : undefined}
                    />
                  </button>
                  <button
                    type="button"
                    className="btn btn-secondary"
                    onClick={() => void handleTopUp()}
                  >
                    Top up credits
                  </button>
                </div>
              </div>
            </div>
          </div>
        </section>
      ) : null}
    </div>
  );
}

function displayName(account: AccountStatus) {
  return (
    account.user?.displayName ??
    (account.user?.handle ? `@${account.user.handle}` : "Signed in")
  );
}

function formatCredits(credits?: number) {
  return (credits ?? 0).toLocaleString();
}

function formatUsd(usdMillis?: number) {
  return `$${((usdMillis ?? 0) / 1000).toFixed(2)}`;
}

function messageFromError(error: unknown) {
  if (error && typeof error === "object" && "message" in error) {
    return String((error as { message: unknown }).message);
  }
  return String(error);
}
