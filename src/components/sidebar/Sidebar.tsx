import { useState, type FormEvent } from "react";
import type { FolderDto } from "../../lib/tauri";

type SidebarProps = {
  folders: FolderDto[];
  selectedFolderId?: string;
  onCreateFolder: (name: string) => Promise<void> | void;
  onSelectAll: () => void;
  onSelectFolder: (folderId: string) => void;
};

export function Sidebar({
  folders,
  selectedFolderId,
  onCreateFolder,
  onSelectAll,
  onSelectFolder,
}: SidebarProps) {
  const [isCreating, setIsCreating] = useState(false);
  const [folderName, setFolderName] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const name = folderName.trim();
    if (!name) return;

    try {
      setIsSubmitting(true);
      await onCreateFolder(name);
      setFolderName("");
      setIsCreating(false);
    } catch {
      return;
    } finally {
      setIsSubmitting(false);
    }
  }

  return (
    <aside className="sidebar">
      <h1>OS Notetaker</h1>
      {isCreating ? (
        <form className="folder-create-form" onSubmit={handleSubmit}>
          <label>
            <span>Folder name</span>
            <input
              autoFocus
              value={folderName}
              onChange={(event) => setFolderName(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Escape") {
                  setFolderName("");
                  setIsCreating(false);
                }
              }}
            />
          </label>
          <div className="folder-create-actions">
            <button type="submit" disabled={!folderName.trim() || isSubmitting}>
              Create
            </button>
            <button
              type="button"
              onClick={() => {
                setFolderName("");
                setIsCreating(false);
              }}
            >
              Cancel
            </button>
          </div>
        </form>
      ) : (
        <button type="button" onClick={() => setIsCreating(true)}>
          + New Folder
        </button>
      )}
      <button
        type="button"
        className={!selectedFolderId ? "active" : undefined}
        onClick={onSelectAll}
      >
        All Notes
      </button>
      <nav className="folder-nav" aria-label="Folders">
        {folders.map((folder) => (
          <button
            key={folder.id}
            type="button"
            className={selectedFolderId === folder.id ? "active" : undefined}
            onClick={() => onSelectFolder(folder.id)}
          >
            {folder.name}
          </button>
        ))}
      </nav>
    </aside>
  );
}
