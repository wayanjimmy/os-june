import { listen } from "@tauri-apps/api/event";
import mascotUrl from "./assets/june-pangolin.svg";
import {
  AGENT_OPEN_EVENT,
  AGENT_SESSIONS_CHANGED_EVENT,
  AGENT_SESSION_STATUS_EVENT,
  type AgentSessionStatusDetail,
  type AgentSessionStatusKind,
  type AgentSessionsChangedDetail,
} from "./lib/agent-events";
import {
  getMascotEnabled,
  MASCOT_ENABLED_KEY,
  MASCOT_VISIBILITY_CHANGED_EVENT,
  type MascotVisibilityChangedDetail,
} from "./lib/mascot-settings";
import {
  mascotHide,
  mascotOpenAgent,
  mascotSetLayout,
  mascotShow,
} from "./lib/tauri";
import type { HermesSessionInfo } from "./lib/tauri";
import "./styles/mascot.css";

type MascotSessionStatus = AgentSessionStatusKind | "idle";

type StatusRecord = AgentSessionStatusDetail & {
  receivedAt: number;
};

type MascotEntry = {
  id: string;
  title: string;
  summary: string;
  status: MascotSessionStatus;
  updatedAt: string;
  session?: HermesSessionInfo;
};

const EXPANDED_KEY = "scribe:mascot:expanded";
const MAX_VISIBLE_CARDS = 3;
const TERMINAL_STATUS_TTL_MS = 10 * 60 * 1000;

const mascot = document.querySelector<HTMLElement>("#mascot");
const stack = document.querySelector<HTMLElement>("#mascot-stack");
const toggle = document.querySelector<HTMLButtonElement>("#mascot-toggle");
const avatar = document.querySelector<HTMLButtonElement>("#mascot-avatar");
const image = document.querySelector<HTMLImageElement>("#mascot-image");

if (image) image.src = mascotUrl;

const state = {
  enabled: getMascotEnabled(),
  expanded: localStorage.getItem(EXPANDED_KEY) === "true",
  sessions: [] as HermesSessionInfo[],
  selectedSessionId: undefined as string | undefined,
  workingSessionIds: new Set<string>(),
  waitingSessionIds: new Set<string>(),
  statusBySessionId: new Map<string, StatusRecord>(),
  pendingStatuses: [] as StatusRecord[],
};

let lastLayoutKey = "";

function applySessionsChanged(detail?: AgentSessionsChangedDetail) {
  if (!detail) return;
  state.sessions = detail.sessions ?? [];
  state.selectedSessionId = detail.selectedSessionId;
  state.workingSessionIds = new Set(detail.workingSessionIds ?? []);
  state.waitingSessionIds = new Set(detail.waitingSessionIds ?? []);
  state.pendingStatuses = state.pendingStatuses.filter(
    (pending) =>
      !state.sessions.some((session) => sameSubject(session, pending)),
  );
  render();
}

function applyStatus(detail?: AgentSessionStatusDetail) {
  if (!detail) return;
  const record: StatusRecord = { ...detail, receivedAt: Date.now() };
  if (detail.sessionId) {
    state.statusBySessionId.set(detail.sessionId, record);
    state.pendingStatuses = state.pendingStatuses.filter(
      (pending) => !sameStatusSubject(pending, record),
    );
  } else {
    const key = statusSubject(record);
    state.pendingStatuses = [
      record,
      ...state.pendingStatuses.filter((item) => statusSubject(item) !== key),
    ].slice(0, MAX_VISIBLE_CARDS);
  }
  if (
    isActiveStatus(detail.status) &&
    localStorage.getItem(EXPANDED_KEY) === null
  ) {
    state.expanded = true;
  }
  pruneOldStatuses();
  render();
}

function applyVisibility(enabled: boolean) {
  state.enabled = enabled;
  render();
  if (enabled) {
    void mascotShow().catch(() => {});
  } else {
    void mascotHide().catch(() => {});
  }
}

function render() {
  if (!mascot || !stack || !toggle) return;

  const realEntries = buildEntries(false);
  const entries = state.expanded ? buildEntries(true) : realEntries;
  const expanded = state.enabled && state.expanded && entries.length > 0;

  mascot.dataset.expanded = expanded ? "true" : "false";
  stack.replaceChildren();
  if (expanded) {
    for (const entry of entries) stack.appendChild(renderCard(entry));
  }
  stack.setAttribute("aria-hidden", expanded ? "false" : "true");
  toggle.setAttribute("aria-expanded", expanded ? "true" : "false");
  toggle.setAttribute(
    "aria-label",
    expanded ? "Collapse June mascot" : "Expand June mascot",
  );

  void syncWindowLayout(expanded, expanded ? entries.length : 0);
}

function renderCard(entry: MascotEntry) {
  const card = document.createElement("article");
  card.className = "mascot-card";
  card.dataset.status = entry.status;

  const body = document.createElement("button");
  body.type = "button";
  body.className = "mascot-card-body";
  body.addEventListener("click", () => {
    void openAgent(entry.session);
  });

  const title = document.createElement("h2");
  title.textContent = entry.title;
  body.appendChild(title);

  const summary = document.createElement("p");
  summary.textContent = entry.summary;
  body.appendChild(summary);

  const status = document.createElement("span");
  status.className = "mascot-status";
  status.dataset.status = entry.status;
  status.setAttribute("aria-hidden", "true");
  body.appendChild(status);
  card.appendChild(body);

  const actions = document.createElement("div");
  actions.className = "mascot-card-actions";

  if (entry.status === "waitingForUser") {
    const reply = document.createElement("button");
    reply.type = "button";
    reply.className = "mascot-reply";
    reply.textContent = "Reply";
    reply.addEventListener("click", () => {
      void openAgent(entry.session);
    });
    actions.appendChild(reply);
  }
  if (actions.childElementCount > 0) card.appendChild(actions);

  const dismiss = document.createElement("button");
  dismiss.type = "button";
  dismiss.className = "mascot-dismiss";
  dismiss.setAttribute("aria-label", "Collapse June mascot");
  dismiss.addEventListener("click", () => setExpanded(false));
  card.appendChild(dismiss);

  return card;
}

function buildEntries(includeIdle: boolean) {
  const now = Date.now();
  const entries: MascotEntry[] = [];
  const seen = new Set<string>();

  for (const session of state.sessions) {
    const id = session.id;
    if (!id || seen.has(id)) continue;
    seen.add(id);
    const record = state.statusBySessionId.get(id);
    if (
      record &&
      isTerminalStatus(record.status) &&
      now - record.receivedAt > TERMINAL_STATUS_TTL_MS
    ) {
      state.statusBySessionId.delete(id);
    }
    entries.push(entryFromSession(session, state.statusBySessionId.get(id)));
  }

  for (const record of state.pendingStatuses) {
    entries.push(entryFromPending(record));
  }

  const sorted = entries.sort(compareEntries).slice(0, MAX_VISIBLE_CARDS);

  if (!sorted.length && includeIdle) {
    sorted.push({
      id: "idle",
      title: "No active sessions",
      summary: "June is ready.",
      status: "idle",
      updatedAt: new Date(0).toISOString(),
    });
  }

  return sorted;
}

function entryFromSession(
  session: HermesSessionInfo,
  record?: StatusRecord,
): MascotEntry {
  const status = sessionStatus(session, record);
  return {
    id: session.id,
    title: sessionTitle(session, record),
    summary: sessionSummary(session, status, record),
    status,
    updatedAt: sessionTimestamp(session, record),
    session,
  };
}

function entryFromPending(record: StatusRecord): MascotEntry {
  return {
    id: `pending:${statusSubject(record)}`,
    title: statusTitle(record),
    summary: statusSummary(record),
    status: record.status,
    updatedAt: new Date(record.receivedAt).toISOString(),
  };
}

function sessionStatus(
  session: HermesSessionInfo,
  record?: StatusRecord,
): MascotSessionStatus {
  if (state.waitingSessionIds.has(session.id)) return "waitingForUser";
  if (state.workingSessionIds.has(session.id)) return "running";
  if (record && Date.now() - record.receivedAt <= TERMINAL_STATUS_TTL_MS) {
    return record.status;
  }
  return "idle";
}

function sessionTitle(session: HermesSessionInfo, record?: StatusRecord) {
  return (
    record?.title?.trim() ||
    session.title?.trim() ||
    session.preview?.trim() ||
    "Agent session"
  );
}

function sessionSummary(
  session: HermesSessionInfo,
  status: MascotSessionStatus,
  record?: StatusRecord,
) {
  const summary = record?.summary?.trim();
  if (summary) return summary;
  if (status !== "idle") return statusLabel(status);
  return session.preview?.trim() || "Idle";
}

function sessionTimestamp(session: HermesSessionInfo, record?: StatusRecord) {
  if (record) return new Date(record.receivedAt).toISOString();
  return (
    session.last_active ??
    session.lastActive ??
    session.started_at ??
    session.startedAt ??
    new Date(0).toISOString()
  );
}

function statusTitle(record: StatusRecord) {
  return record.title?.trim() || record.prompt?.trim() || "Agent session";
}

function statusSummary(record: StatusRecord) {
  return record.summary?.trim() || statusLabel(record.status);
}

function statusLabel(status: MascotSessionStatus) {
  switch (status) {
    case "received":
    case "starting":
    case "running":
      return "Thinking";
    case "waitingForUser":
      return "Needs input";
    case "completed":
      return "Done";
    case "failed":
      return "Hit a problem";
    case "cancelled":
      return "Stopped";
    case "idle":
      return "Idle";
  }
}

function compareEntries(a: MascotEntry, b: MascotEntry) {
  const rank = statusRank(a.status) - statusRank(b.status);
  if (rank !== 0) return rank;
  return b.updatedAt.localeCompare(a.updatedAt);
}

function statusRank(status: MascotSessionStatus) {
  if (status === "waitingForUser") return 0;
  if (status === "received" || status === "starting" || status === "running")
    return 1;
  if (status === "failed") return 2;
  if (status === "completed" || status === "cancelled") return 3;
  return 4;
}

function isActiveStatus(status: AgentSessionStatusKind) {
  return (
    status === "received" ||
    status === "starting" ||
    status === "running" ||
    status === "waitingForUser"
  );
}

function isTerminalStatus(status: AgentSessionStatusKind) {
  return (
    status === "completed" || status === "failed" || status === "cancelled"
  );
}

function pruneOldStatuses() {
  const now = Date.now();
  state.pendingStatuses = state.pendingStatuses.filter(
    (record) =>
      !isTerminalStatus(record.status) ||
      now - record.receivedAt <= TERMINAL_STATUS_TTL_MS,
  );
  for (const [id, record] of state.statusBySessionId) {
    if (
      isTerminalStatus(record.status) &&
      now - record.receivedAt > TERMINAL_STATUS_TTL_MS
    ) {
      state.statusBySessionId.delete(id);
    }
  }
}

function sameSubject(session: HermesSessionInfo, record: StatusRecord) {
  const title = statusSubject(record);
  return (
    session.id === record.sessionId ||
    session.title?.trim().toLowerCase() === title
  );
}

function sameStatusSubject(a: StatusRecord, b: StatusRecord) {
  return statusSubject(a) === statusSubject(b);
}

function statusSubject(record: StatusRecord) {
  return statusTitle(record).trim().toLowerCase();
}

async function syncWindowLayout(expanded: boolean, cardCount: number) {
  const key = `${state.enabled}:${expanded}:${cardCount}`;
  if (key === lastLayoutKey) return;
  lastLayoutKey = key;
  if (!state.enabled) {
    await mascotHide().catch(() => {});
    return;
  }
  await mascotSetLayout({ expanded, cardCount }).catch(() => {});
  await mascotShow().catch(() => {});
}

function setExpanded(expanded: boolean) {
  state.expanded = expanded;
  localStorage.setItem(EXPANDED_KEY, expanded ? "true" : "false");
  render();
}

async function openAgent(session?: HermesSessionInfo) {
  await mascotOpenAgent(session).catch(() => {
    window.dispatchEvent(
      new CustomEvent(AGENT_OPEN_EVENT, {
        detail: { session },
      }),
    );
  });
}

toggle?.addEventListener("click", () => setExpanded(!state.expanded));
avatar?.addEventListener("click", () => {
  const [entry] = buildEntries(false);
  void openAgent(entry?.session);
});

window.addEventListener(MASCOT_VISIBILITY_CHANGED_EVENT, (event) => {
  const detail = (event as CustomEvent<MascotVisibilityChangedDetail>).detail;
  if (detail) applyVisibility(detail.enabled);
});

window.addEventListener(AGENT_SESSIONS_CHANGED_EVENT, (event) => {
  applySessionsChanged(
    (event as CustomEvent<AgentSessionsChangedDetail>).detail,
  );
});

window.addEventListener(AGENT_SESSION_STATUS_EVENT, (event) => {
  applyStatus((event as CustomEvent<AgentSessionStatusDetail>).detail);
});

window.addEventListener("storage", (event) => {
  if (event.key === MASCOT_ENABLED_KEY) {
    applyVisibility(event.newValue !== "false");
  }
});

void listen<AgentSessionsChangedDetail>(AGENT_SESSIONS_CHANGED_EVENT, (event) =>
  applySessionsChanged(event.payload),
).catch(() => {});

void listen<AgentSessionStatusDetail>(AGENT_SESSION_STATUS_EVENT, (event) =>
  applyStatus(event.payload),
).catch(() => {});

void listen<MascotVisibilityChangedDetail>(
  MASCOT_VISIBILITY_CHANGED_EVENT,
  (event) => applyVisibility(event.payload.enabled),
).catch(() => {});

render();
applyVisibility(state.enabled);
