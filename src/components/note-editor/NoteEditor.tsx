import { Mic } from "lucide-react";
import type {
  FolderDto,
  NoteDto,
  RecordingSourceMode,
  RecordingSourceReadinessDto,
  RecordingStatusDto,
} from "../../lib/tauri";
import { RecorderBar } from "../recorder/RecorderBar";
import { SourceModeControl } from "../recorder/SourceModeControl";
import { FolderPicker } from "./FolderPicker";

type NoteEditorProps = {
  note: NoteDto;
  folders: FolderDto[];
  recordingStatus?: RecordingStatusDto;
  sourceMode: RecordingSourceMode;
  sourceReadiness?: RecordingSourceReadinessDto;
  checkingSourceReadiness: boolean;
  onTitleChange: (title: string) => void;
  onContentChange: (content: string) => void;
  onSourceModeChange: (mode: RecordingSourceMode) => void;
  onStartRecording: () => void;
  onPauseRecording: (sessionId: string) => void;
  onResumeRecording: (sessionId: string) => void;
  onFinishRecording: (sessionId: string) => void;
  onRetry: () => void;
  onAssignFolder: (folderId: string) => void;
  onRemoveFolder: (folderId: string) => void;
  onTabChange: (tab: "notes" | "transcription") => void;
};

export function NoteEditor({
  note,
  folders,
  recordingStatus,
  sourceMode,
  sourceReadiness,
  checkingSourceReadiness,
  onTitleChange,
  onContentChange,
  onSourceModeChange,
  onStartRecording,
  onPauseRecording,
  onResumeRecording,
  onFinishRecording,
  onRetry,
  onAssignFolder,
  onRemoveFolder,
  onTabChange,
}: NoteEditorProps) {
  const content = note.editedContent ?? note.generatedContent ?? "";
  const activeTab = note.activeTab ?? "notes";

  return (
    <article className="note-editor">
      <input
        className="note-title"
        aria-label="Note title"
        placeholder="New note"
        value={note.title}
        onChange={(event) => onTitleChange(event.currentTarget.value)}
      />
      <div className="tabs" role="tablist" aria-label="Note views">
        <button
          type="button"
          role="tab"
          aria-selected={activeTab === "notes"}
          onClick={() => onTabChange("notes")}
        >
          Notes
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={activeTab === "transcription"}
          onClick={() => onTabChange("transcription")}
        >
          Transcription
        </button>
      </div>
      <FolderPicker
        folders={folders}
        folderIds={note.folderIds}
        onAssign={onAssignFolder}
        onRemove={onRemoveFolder}
      />
      {activeTab === "transcription" ? (
        <section className="transcript-view">
          {note.sourceTranscripts?.length ? (
            <div className="source-transcripts">
              {note.sourceTranscripts.map((transcript) => (
                <section key={transcript.id}>
                  <h3>
                    {transcript.source === "system"
                      ? "System audio"
                      : "Microphone"}
                  </h3>
                  <p>{transcript.text}</p>
                  {transcript.lastError ? <p>{transcript.lastError}</p> : null}
                </section>
              ))}
            </div>
          ) : note.transcript?.text ? (
            <p>{note.transcript.text}</p>
          ) : (
            <div className="empty-state">
              <p>{note.lastError ?? "No transcript is available yet."}</p>
              {note.audio ? (
                <button type="button" onClick={onRetry}>
                  Retry
                </button>
              ) : null}
            </div>
          )}
        </section>
      ) : (
        <textarea
          className="note-body"
          aria-label="Generated note"
          placeholder="Record a voice note to generate notes here."
          value={content}
          onChange={(event) => onContentChange(event.currentTarget.value)}
        />
      )}
      <div className="editor-footer">
        <SourceModeControl
          value={sourceMode}
          disabled={!!recordingStatus}
          readiness={sourceReadiness}
          onChange={onSourceModeChange}
        />
        {recordingStatus ? (
          <RecorderBar
            status={recordingStatus}
            onPause={onPauseRecording}
            onResume={onResumeRecording}
            onDone={onFinishRecording}
          />
        ) : (
          <button
            type="button"
            className="record-button"
            disabled={
              checkingSourceReadiness ||
              sourceReadiness?.sources.some(
                (source) => source.required && !source.ready,
              )
            }
            onClick={onStartRecording}
          >
            <Mic size={18} />
            {checkingSourceReadiness ? "Checking..." : "Record"}
          </button>
        )}
      </div>
    </article>
  );
}
