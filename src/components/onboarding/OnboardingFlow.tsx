import { useEffect, useMemo, useState } from "react";
import {
  isDataSharingEnabled,
  loadOnboardingProfile,
  saveOnboardingProfile,
  setAgentRiskAcknowledged,
  setDataSharingEnabled,
  type OnboardingProfile,
} from "../../lib/onboarding";
import { dictationSettings } from "../../lib/tauri";
import type { AccountStatus } from "../../lib/tauri";
import { FinishStep } from "./steps/FinishStep";
import {
  AgentHonestyStep,
  AgentIntroStep,
  DictationPracticeStep,
  MeetingNotesStep,
} from "./steps/LearnSteps";
import {
  AccessibilityStep,
  MicrophoneStep,
  PermissionsRecapStep,
} from "./steps/PermissionSteps";
import { DataSharingStep, PrivacyStep } from "./steps/PrivacySteps";
import { SetupStep } from "./steps/SetupStep";
import { FocusStep, RoleStep, WelcomeStep } from "./steps/WelcomeSteps";
import { usePermissionStatuses } from "./use-permission-status";

type StepId =
  | "welcome"
  | "role"
  | "focus"
  | "privacy"
  | "data-sharing"
  | "microphone"
  | "accessibility"
  | "permissions-recap"
  | "setup"
  | "dictation-practice"
  | "meeting-notes"
  | "agent-intro"
  | "agent-honesty"
  | "finish";

const STAGES = [
  "Welcome",
  "Privacy",
  "Permissions",
  "Set up",
  "Learn",
  "Finish",
] as const;

const STEPS: { id: StepId; stage: (typeof STAGES)[number] }[] = [
  { id: "welcome", stage: "Welcome" },
  { id: "role", stage: "Welcome" },
  { id: "focus", stage: "Welcome" },
  { id: "privacy", stage: "Privacy" },
  { id: "data-sharing", stage: "Privacy" },
  { id: "microphone", stage: "Permissions" },
  { id: "accessibility", stage: "Permissions" },
  { id: "permissions-recap", stage: "Permissions" },
  { id: "setup", stage: "Set up" },
  { id: "dictation-practice", stage: "Learn" },
  { id: "meeting-notes", stage: "Learn" },
  { id: "agent-intro", stage: "Learn" },
  { id: "agent-honesty", stage: "Learn" },
  { id: "finish", stage: "Finish" },
];

type Props = {
  account: AccountStatus;
  onComplete: () => void;
};

export function OnboardingFlow({ account, onComplete }: Props) {
  const [stepIndex, setStepIndex] = useState(0);
  const [profile, setProfile] = useState<OnboardingProfile>(() =>
    loadOnboardingProfile(),
  );
  const [dataSharing, setDataSharing] = useState(() => isDataSharingEnabled());
  const [shortcutLabel, setShortcutLabel] = useState("fn");
  const [language, setLanguage] = useState("");

  const step = STEPS[stepIndex];
  const stageIndex = STAGES.indexOf(step.stage);

  // Permission state powers three screens; only poll the helper while the
  // user is actually on one of them.
  const permissionStepActive =
    step.id === "microphone" ||
    step.id === "accessibility" ||
    step.id === "permissions-recap";
  const permissionStatuses = usePermissionStatuses(permissionStepActive);

  useEffect(() => {
    dictationSettings()
      .then(({ settings }) => {
        if (settings.pushToTalkShortcut.label) {
          setShortcutLabel(settings.pushToTalkShortcut.label);
        }
        setLanguage(settings.language ?? "");
      })
      .catch(() => undefined);
  }, []);

  const firstName = useMemo(() => {
    const display = account.user?.displayName ?? account.user?.handle;
    return display?.split(/\s+/)[0];
  }, [account.user?.displayName, account.user?.handle]);

  function handleProfileChange(next: OnboardingProfile) {
    setProfile(next);
    saveOnboardingProfile(next);
  }

  function handleDataSharingChange(enabled: boolean) {
    setDataSharing(enabled);
    setDataSharingEnabled(enabled);
  }

  function goNext() {
    setStepIndex((index) => Math.min(index + 1, STEPS.length - 1));
  }

  function goBack() {
    setStepIndex((index) => Math.max(index - 1, 0));
  }

  return (
    <div className="onboarding-screen">
      <nav className="onboarding-progress" aria-label="Setup progress">
        {STAGES.map((stage, index) => (
          <span
            key={stage}
            className="onboarding-progress-stage"
            data-state={
              index < stageIndex
                ? "done"
                : index === stageIndex
                  ? "current"
                  : "upcoming"
            }
            aria-current={index === stageIndex ? "step" : undefined}
          >
            {stage}
          </span>
        ))}
      </nav>
      <div className="onboarding-body">
        {stepIndex > 0 ? (
          <button
            type="button"
            className="onboarding-back"
            onClick={goBack}
            aria-label="Back"
          >
            ← Back
          </button>
        ) : null}
        {step.id === "welcome" ? (
          <WelcomeStep name={firstName} onContinue={goNext} />
        ) : step.id === "role" ? (
          <RoleStep
            profile={profile}
            onProfileChange={handleProfileChange}
            onContinue={goNext}
          />
        ) : step.id === "focus" ? (
          <FocusStep
            profile={profile}
            onProfileChange={handleProfileChange}
            onContinue={goNext}
          />
        ) : step.id === "privacy" ? (
          <PrivacyStep onContinue={goNext} />
        ) : step.id === "data-sharing" ? (
          <DataSharingStep
            enabled={dataSharing}
            onEnabledChange={handleDataSharingChange}
            onContinue={goNext}
          />
        ) : step.id === "microphone" ? (
          <MicrophoneStep statuses={permissionStatuses} onContinue={goNext} />
        ) : step.id === "accessibility" ? (
          <AccessibilityStep
            statuses={permissionStatuses}
            onContinue={goNext}
          />
        ) : step.id === "permissions-recap" ? (
          <PermissionsRecapStep
            statuses={permissionStatuses}
            onContinue={goNext}
          />
        ) : step.id === "setup" ? (
          <SetupStep
            shortcutLabel={shortcutLabel}
            language={language}
            onLanguageChange={setLanguage}
            onContinue={goNext}
          />
        ) : step.id === "dictation-practice" ? (
          <DictationPracticeStep
            name={firstName}
            shortcutLabel={shortcutLabel}
            onContinue={goNext}
          />
        ) : step.id === "meeting-notes" ? (
          <MeetingNotesStep onContinue={goNext} />
        ) : step.id === "agent-intro" ? (
          <AgentIntroStep onContinue={goNext} />
        ) : step.id === "agent-honesty" ? (
          <AgentHonestyStep
            onAcknowledged={() => setAgentRiskAcknowledged(true)}
            onContinue={goNext}
          />
        ) : (
          <FinishStep shortcutLabel={shortcutLabel} onComplete={onComplete} />
        )}
      </div>
    </div>
  );
}
