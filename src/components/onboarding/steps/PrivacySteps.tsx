import { Switch } from "../../ui/Switch";
import { StepActions, StepHeading } from "../StepChrome";

const PRIVACY_CARDS = [
  {
    title: "Local by default",
    body: "The agent runs on your Mac, built on open-source Hermes. Your files, sessions, and memory stay on your disk — never mirrored to a cloud.",
  },
  {
    title: "Private inference",
    body: "Prompts leave your Mac only for model inference, routed to zero-retention models by default. Nothing stored, nothing trained on. Ever.",
  },
  {
    title: "Verifiable",
    body: "Our code is open source and our backend runs in a secure enclave. You don't have to trust us — you can check.",
  },
];

export function PrivacyStep({ onContinue }: { onContinue: () => void }) {
  return (
    <section className="onboarding-step">
      <StepHeading
        title="Private by architecture, not by promise"
        subtitle="Every layer of June defaults to private. The ones that matter most, you can verify."
      />
      <div className="onboarding-card-grid">
        {PRIVACY_CARDS.map((card) => (
          <article key={card.title} className="onboarding-info-card">
            <h2>{card.title}</h2>
            <p>{card.body}</p>
          </article>
        ))}
      </div>
      <p className="onboarding-footnote">
        <a
          href="https://opensoftware.network/privacy"
          target="_blank"
          rel="noreferrer"
        >
          Verify it yourself
        </a>{" "}
        — how routing, retention, and attestation work.
      </p>
      <StepActions onContinue={onContinue} />
    </section>
  );
}

export function DataSharingStep({
  enabled,
  onEnabledChange,
  onContinue,
}: {
  enabled: boolean;
  onEnabledChange: (enabled: boolean) => void;
  onContinue: () => void;
}) {
  return (
    <section className="onboarding-step">
      <StepHeading
        title="Choose your data sharing preference"
        subtitle="Off by default. June works exactly the same either way."
      />
      <div className="onboarding-setting-card">
        <div className="onboarding-setting-copy">
          <h2 id="onboarding-data-sharing-label">Usage analytics</h2>
          <p>Share anonymized usage data to help improve June.</p>
        </div>
        <Switch
          checked={enabled}
          onCheckedChange={onEnabledChange}
          aria-labelledby="onboarding-data-sharing-label"
        />
      </div>
      <p className="onboarding-footnote">
        Either way: we store only your account, login, and billing records. Your
        prompts, transcripts, files, and memory are not on that list. Change
        this anytime in Settings.
      </p>
      <StepActions onContinue={onContinue} />
    </section>
  );
}
