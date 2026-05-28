import { useCallback, useState } from "react";
import { Dialog } from "../ui/Dialog";
import {
  osAccountsCancelLogin,
  osAccountsLogin,
} from "../../lib/tauri";
import type { AccountStatus } from "../../lib/tauri";

export type SignInPromptReason = "record" | "dictate";

type Props = {
  open: boolean;
  reason: SignInPromptReason;
  onClose: () => void;
  onSignedIn: (account: AccountStatus) => void;
};

const COPY: Record<
  SignInPromptReason,
  { title: string; description: string }
> = {
  record: {
    title: "Sign in to record",
    description:
      "Recording a note runs through Scribe API, which needs an Open Software account for transcription and note generation.",
  },
  dictate: {
    title: "Sign in to dictate",
    description:
      "Dictation sends audio to Scribe API for transcription. Sign in with Open Software to enable it.",
  },
};

export function SignInPrompt({ open, reason, onClose, onSignedIn }: Props) {
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<string>();
  const copy = COPY[reason];

  async function handleSignIn() {
    setBusy(true);
    setStatus(undefined);
    try {
      const next = await osAccountsLogin();
      if (next.signedIn) {
        onSignedIn(next);
      } else {
        setStatus("Sign-in did not complete. Please try again.");
      }
    } catch (error) {
      setStatus(messageFromError(error));
    } finally {
      setBusy(false);
    }
  }

  const cancelInFlight = useCallback(async () => {
    try {
      await osAccountsCancelLogin();
    } catch {
      // The pending osAccountsLogin promise will reject with "login_canceled"
      // and handleSignIn's catch surfaces the message — nothing to do here.
    }
  }, []);

  async function handleSecondary() {
    if (busy) {
      await cancelInFlight();
      return;
    }
    onClose();
  }

  const handleDismiss = useCallback(() => {
    // The Dialog primitive's X button and Esc both come through here; if a
    // login is in flight we must tell the Rust side to stop listening on the
    // loopback port before unmounting, otherwise the next attempt sees the
    // port still bound.
    if (busy) {
      void cancelInFlight();
      return;
    }
    onClose();
  }, [busy, cancelInFlight, onClose]);

  return (
    <Dialog
      open={open}
      onClose={handleDismiss}
      title={copy.title}
      description={copy.description}
      width={480}
      className="sign-in-prompt-dialog"
      disableBackdropClose
      footer={
        <>
          <button
            type="button"
            className="btn btn-ghost"
            onClick={() => void handleSecondary()}
          >
            {busy ? "Cancel" : "Not now"}
          </button>
          {busy ? null : (
            <button
              type="button"
              className="btn btn-secondary"
              onClick={() => void handleSignIn()}
            >
              Sign in with Open Software
            </button>
          )}
        </>
      }
    >
      {busy ? (
        <div
          className="sign-in-prompt-busy"
          role="status"
          aria-live="polite"
        >
          <span className="sign-in-prompt-spinner" aria-hidden />
          <p>
            Waiting for sign-in to complete in your browser. You can keep this
            window open — Scribe will pick up the callback automatically.
          </p>
        </div>
      ) : status ? (
        <p className="sign-in-prompt-status">{status}</p>
      ) : null}
    </Dialog>
  );
}

function messageFromError(error: unknown) {
  if (error && typeof error === "object" && "message" in error) {
    return String((error as { message: unknown }).message);
  }
  return String(error);
}
