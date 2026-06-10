import { LANGUAGE_OPTIONS } from "../../../lib/dictation-languages";
import { setDictationLanguage } from "../../../lib/tauri";
import { StepActions, StepHeading } from "../StepChrome";

export function SetupStep({
  shortcutLabel,
  language,
  onLanguageChange,
  onContinue,
}: {
  shortcutLabel: string;
  language: string;
  onLanguageChange: (language: string) => void;
  onContinue: () => void;
}) {
  return (
    <section className="onboarding-step">
      <StepHeading
        title="Set up dictation"
        subtitle="Two things and you're ready to talk."
      />
      <div className="onboarding-setting-card">
        <div className="onboarding-setting-copy">
          <h2>Your dictation key</h2>
          <p>
            June starts listening when you hold{" "}
            <kbd className="onboarding-kbd">{shortcutLabel}</kbd> and types what
            you said when you let go. Change it anytime in Settings.
          </p>
        </div>
      </div>
      <div className="onboarding-setting-card">
        <div className="onboarding-setting-copy">
          <h2 id="onboarding-language-label">Language</h2>
          <p>June understands you in 20+ languages.</p>
        </div>
        <select
          className="onboarding-select"
          aria-labelledby="onboarding-language-label"
          value={language}
          onChange={(event) => {
            const next = event.target.value;
            onLanguageChange(next);
            void setDictationLanguage(next || undefined).catch(() => undefined);
          }}
        >
          {LANGUAGE_OPTIONS.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </select>
      </div>
      <StepActions onContinue={onContinue} />
    </section>
  );
}
