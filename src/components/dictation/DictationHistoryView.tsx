import { listen } from "@tauri-apps/api/event";
import { IconClipboard } from "central-icons/IconClipboard";
import { IconClock } from "central-icons/IconClock";
import { IconMagnifyingGlass } from "central-icons/IconMagnifyingGlass";
import { IconMicrophone } from "central-icons/IconMicrophone";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  dictationSettings,
  listDictationHistory,
  type DictationSettingsDto,
  type DictationHistoryItemDto,
} from "../../lib/tauri";

type HistoryGroup = {
  label: string;
  items: DictationHistoryItemDto[];
};

export function DictationHistoryView() {
  const [items, setItems] = useState<DictationHistoryItemDto[]>([]);
  const [retentionDays, setRetentionDays] = useState(7);
  const [query, setQuery] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [settings, setSettings] = useState<DictationSettingsDto>();

  const loadHistory = useCallback(async () => {
    try {
      const response = await listDictationHistory();
      setItems(response.items);
      setRetentionDays(response.retentionDays);
      setError(null);
    } catch (err) {
      setError(messageFromError(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadHistory();
  }, [loadHistory]);

  useEffect(() => {
    dictationSettings()
      .then((response) => setSettings(response.settings))
      .catch(() => undefined);
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<string>("dictation-event", (event) => {
      const payload = parseDictationEvent(event.payload);
      if (payload?.type === "final_transcript") {
        void loadHistory();
      }
    }).then((cleanup) => {
      unlisten = cleanup;
    });
    return () => unlisten?.();
  }, [loadHistory]);

  const filtered = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    if (!normalized) return items;
    return items.filter((item) =>
      `${item.text} ${item.provider} ${item.language ?? ""}`
        .toLowerCase()
        .includes(normalized),
    );
  }, [items, query]);

  const groups = useMemo(() => groupHistoryItems(filtered), [filtered]);

  async function copyDictation(item: DictationHistoryItemDto) {
    const text = item.text.trim();
    if (!text) return;
    await navigator.clipboard.writeText(`${text} `);
    setCopiedId(item.id);
    window.setTimeout(() => setCopiedId(null), 1200);
  }

  return (
    <section className="dictation-history-workspace" aria-label="Dictation">
      <header className="folders-header">
        <div className="folders-heading">
          <h1>Dictation</h1>
          <p className="folders-subtitle">
            Last {retentionDays} days · {items.length}{" "}
            {items.length === 1 ? "dictation" : "dictations"}
          </p>
          <p className="dictation-shortcut-hint">
            Hold {settings?.pushToTalkShortcut.label ?? "Fn"} to dictate, or
            press {settings?.toggleShortcut.label ?? "Ctrl+Opt+Space"} to start
            or stop.
          </p>
        </div>
      </header>

      {items.length > 0 ? (
        <div className="folders-controls">
          <label className="folders-search">
            <IconMagnifyingGlass size={14} />
            <input
              type="search"
              placeholder="Search"
              value={query}
              onChange={(event) => setQuery(event.currentTarget.value)}
            />
          </label>
        </div>
      ) : null}

      {error ? <p className="error-banner">{error}</p> : null}

      {loading ? (
        <div className="folders-empty">
          <p>Loading dictations…</p>
        </div>
      ) : items.length === 0 ? (
        <div className="folders-empty">
          <IconMicrophone size={22} />
          <p>No dictations yet.</p>
        </div>
      ) : groups.length === 0 ? (
        <div className="folders-empty">
          <p>No dictations match “{query.trim()}”.</p>
        </div>
      ) : (
        <div className="dictation-history-groups">
          {groups.map((group) => (
            <section className="dictation-history-group" key={group.label}>
              <h2>{group.label}</h2>
              <ul className="dictation-history-list" role="list">
                {group.items.map((item) => (
                  <li className="dictation-history-item" key={item.id}>
                    <div className="dictation-history-meta">
                      <span>
                        <IconClock size={13} />
                        {formatTime(item.createdAt)}
                      </span>
                      {item.language ? <span>{item.language}</span> : null}
                    </div>
                    <p>{item.text}</p>
                    <button
                      type="button"
                      className="dictation-copy"
                      onClick={() => void copyDictation(item)}
                    >
                      <IconClipboard size={14} />
                      {copiedId === item.id ? "Copied" : "Copy"}
                    </button>
                  </li>
                ))}
              </ul>
            </section>
          ))}
        </div>
      )}
    </section>
  );
}

function groupHistoryItems(items: DictationHistoryItemDto[]): HistoryGroup[] {
  const groups: HistoryGroup[] = [];
  for (const item of items) {
    const label = formatGroupLabel(item.createdAt);
    const group = groups.find((candidate) => candidate.label === label);
    if (group) {
      group.items.push(item);
    } else {
      groups.push({ label, items: [item] });
    }
  }
  return groups;
}

function formatGroupLabel(iso: string) {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "Earlier";
  const now = new Date();
  if (isSameDate(date, now)) return "Today";
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (isSameDate(date, yesterday)) return "Yesterday";
  return date.toLocaleDateString(undefined, {
    weekday: "long",
    month: "short",
    day: "numeric",
  });
}

function formatTime(iso: string) {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleTimeString(undefined, {
    hour: "numeric",
    minute: "2-digit",
  });
}

function isSameDate(left: Date, right: Date) {
  return (
    left.getFullYear() === right.getFullYear() &&
    left.getMonth() === right.getMonth() &&
    left.getDate() === right.getDate()
  );
}

function parseDictationEvent(payload: unknown): { type?: string } | undefined {
  try {
    if (typeof payload === "string")
      return JSON.parse(payload) as { type?: string };
    if (payload && typeof payload === "object")
      return payload as { type?: string };
  } catch {
    return undefined;
  }
  return undefined;
}

function messageFromError(err: unknown) {
  if (err && typeof err === "object" && "message" in err) {
    const message = (err as { message?: unknown }).message;
    if (typeof message === "string") return message;
  }
  return "Dictation history is unavailable.";
}
