import { useEffect } from "react";
import { IconCircleCheck } from "central-icons/IconCircleCheck";
import {
  dictationHelperCommand,
  openPrivacySettings,
} from "../../../lib/tauri";
import { StepActions, StepHeading } from "../StepChrome";
import {
  isAccessibilityGranted,
  isMicrophoneDenied,
  isMicrophoneGranted,
  type PermissionStatuses,
} from "../use-permission-status";

function PermissionCard({
  granted,
  title,
  body,
  action,
}: {
  granted: boolean;
  title: string;
  body: string;
  action?: { label: string; onClick: () => void };
}) {
  return (
    <div className="onboarding-permission-card" data-granted={granted}>
      <div className="onboarding-permission-copy">
        <h2>
          {title}
          {granted ? (
            <span className="onboarding-permission-check" aria-label="Granted">
              <IconCircleCheck size={16} aria-hidden />
            </span>
          ) : null}
        </h2>
        <p>{body}</p>
      </div>
      {!granted && action ? (
        <button
          type="button"
          className="primary-action"
          onClick={action.onClick}
        >
          {action.label}
        </button>
      ) : null}
    </div>
  );
}

export function MicrophoneStep({
  statuses,
  onContinue,
}: {
  statuses: PermissionStatuses;
  onContinue: () => void;
}) {
  const granted = isMicrophoneGranted(statuses);
  const denied = isMicrophoneDenied(statuses);

  // Fire the native TCC prompt as soon as the screen shows — the user just
  // read why we're asking, so the dialog lands in context. No-op when
  // already granted; for already-denied users the helper emits the current
  // status so the System Settings fallback renders instead.
  useEffect(() => {
    void dictationHelperCommand({
      type: "request_microphone_permission",
    }).catch(() => undefined);
  }, []);

  return (
    <section className="onboarding-step">
      <StepHeading
        title="Allow June to use your microphone"
        subtitle="June only listens while you hold the dictation key or while a meeting note is recording."
      />
      <PermissionCard
        granted={granted}
        title={granted ? "June can use your microphone" : "Microphone access"}
        body={
          granted
            ? "You're set — dictation and meeting notes can hear you."
            : denied
              ? "Microphone access is turned off for June. Flip the toggle in System Settings, then come back — we'll notice."
              : "macOS will show a one-time prompt. Choose Allow."
        }
        action={
          denied
            ? {
                label: "Open System Settings",
                onClick: () => void openPrivacySettings("microphone"),
              }
            : {
                label: "Allow microphone",
                onClick: () =>
                  void dictationHelperCommand({
                    type: "request_microphone_permission",
                  }).catch(() => undefined),
              }
        }
      />
      <StepActions
        onContinue={onContinue}
        continueDisabled={!granted}
        onSkip={onContinue}
      />
    </section>
  );
}

export function AccessibilityStep({
  statuses,
  onContinue,
}: {
  statuses: PermissionStatuses;
  onContinue: () => void;
}) {
  const granted = isAccessibilityGranted(statuses);

  function openAccessibilitySettings() {
    // Fire the helper's prompting check first: it registers the dictation
    // helper in the Accessibility list (so there's a toggle to flip) and
    // shows the native dialog. Open the pane only after that IPC resolves —
    // sequenced, not concurrent, so the registration lands before System
    // Settings can steal focus from the prompt. (Same dance as
    // PermissionBanner.)
    void dictationHelperCommand({ type: "request_accessibility_permission" })
      .catch(() => undefined)
      .finally(() => {
        void openPrivacySettings("accessibility");
      });
  }

  return (
    <section className="onboarding-step">
      <StepHeading
        title="Allow June to type for you"
        subtitle="This lets June put your spoken words into whatever app you're using — your editor, your email, your terminal."
      />
      <PermissionCard
        granted={granted}
        title={granted ? "June can type anywhere" : "Accessibility access"}
        body={
          granted
            ? "You're set — dictation will land at your cursor in any app."
            : "Turn on June in System Settings → Privacy & Security → Accessibility, then come back — we'll notice."
        }
        action={{
          label: "Open System Settings",
          onClick: openAccessibilitySettings,
        }}
      />
      <StepActions
        onContinue={onContinue}
        continueDisabled={!granted}
        onSkip={onContinue}
      />
    </section>
  );
}

export function PermissionsRecapStep({
  statuses,
  onContinue,
}: {
  statuses: PermissionStatuses;
  onContinue: () => void;
}) {
  return (
    <section className="onboarding-step">
      <StepHeading
        title="Thanks for trusting us — here's the full picture"
        subtitle="What June can do now, and the two asks that come later, when they make sense."
      />
      <div className="onboarding-permission-stack">
        <PermissionCard
          granted={isMicrophoneGranted(statuses)}
          title="June can use your microphone"
          body="Only while you hold the dictation key or a meeting note is recording."
        />
        <PermissionCard
          granted={isAccessibilityGranted(statuses)}
          title="June can type anywhere"
          body="Your spoken words land at your cursor, in any app."
        />
        <div className="onboarding-permission-card" data-deferred="true">
          <div className="onboarding-permission-copy">
            <h2>System audio — later</h2>
            <p>
              macOS will ask the first time you record a meeting, so the request
              makes sense when you see it.
            </p>
          </div>
        </div>
        <div className="onboarding-permission-card" data-deferred="true">
          <div className="onboarding-permission-copy">
            <h2>Your files — later</h2>
            <p>The agent asks before it touches anything. Always.</p>
          </div>
        </div>
      </div>
      <StepActions onContinue={onContinue} />
    </section>
  );
}
