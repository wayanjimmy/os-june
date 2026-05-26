import { IconRecord } from "central-icons/IconRecord";
import type { RecoverableRecordingDto } from "../../lib/tauri";

type NoteRecoveryPromptProps = {
  recovery: RecoverableRecordingDto;
  onRecover: (sessionId: string) => void;
  onDiscard: (sessionId: string) => void;
  disabled?: boolean;
};

export function NoteRecoveryPrompt({
  recovery,
  onRecover,
  onDiscard,
  disabled,
}: NoteRecoveryPromptProps) {
  return (
    <section
      className="note-recovery-prompt"
      aria-label="Recoverable recording"
    >
      <p className="note-recovery-prompt-message">
        <span className="note-recovery-prompt-eyebrow">
          <IconRecord size={14} aria-hidden />
          Interrupted recording
        </span>
        <span className="note-recovery-prompt-body">
          We saved {formatBytes(recovery.bytesFound)} before this note stopped.
        </span>
      </p>
      <div className="note-recovery-prompt-actions">
        <button
          type="button"
          className="btn btn-ghost"
          disabled={disabled}
          onClick={() => onDiscard(recovery.sessionId)}
        >
          Discard
        </button>
        <button
          type="button"
          className="btn btn-secondary"
          disabled={disabled}
          onClick={() => onRecover(recovery.sessionId)}
        >
          Recover
        </button>
      </div>
    </section>
  );
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
