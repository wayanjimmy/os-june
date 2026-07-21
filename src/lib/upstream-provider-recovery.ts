import type { AgentChatTurn } from "./agent-chat-runtime";

export type UpstreamProviderRecoveryStore = {
  attempted(storedSessionId: string, recoveryId: string): boolean;
  reserve(storedSessionId: string, recoveryId: string): boolean;
  release(storedSessionId: string, recoveryId: string): void;
  subscribe(listener: () => void): () => void;
  getVersion(): number;
};

function recoveryKey(storedSessionId: string, recoveryId: string) {
  return `${storedSessionId}\u0000${recoveryId}`;
}

/**
 * Returns a stable identity for one upstream-provider failure within a stored
 * session. Live turn ids include client receive times and can differ between
 * the workspace and note-chat gateways, while the failure ordinal is shared
 * by both views and remains stable when persisted messages replace live ones.
 */
export function upstreamProviderRecoveryIds(turns: AgentChatTurn[]): Map<string, string> {
  const ids = new Map<string, string>();
  let ordinal = 0;
  for (const turn of turns) {
    const isProviderFailure = turn.parts.some(
      (part) => part.type === "notice" && part.kind === "upstream-provider",
    );
    if (!isProviderFailure) continue;
    ordinal += 1;
    ids.set(turn.id, `upstream-provider:${ordinal}`);
  }
  return ids;
}

export function createUpstreamProviderRecoveryStore(): UpstreamProviderRecoveryStore {
  const attempts = new Set<string>();
  const listeners = new Set<() => void>();
  let version = 0;

  function emit() {
    version += 1;
    for (const listener of listeners) listener();
  }

  return {
    attempted(storedSessionId, recoveryId) {
      return attempts.has(recoveryKey(storedSessionId, recoveryId));
    },
    reserve(storedSessionId, recoveryId) {
      const key = recoveryKey(storedSessionId, recoveryId);
      if (attempts.has(key)) return false;
      attempts.add(key);
      emit();
      return true;
    },
    release(storedSessionId, recoveryId) {
      if (!attempts.delete(recoveryKey(storedSessionId, recoveryId))) return;
      emit();
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    getVersion() {
      return version;
    },
  };
}

/** Process-local by design: app-restart durability is outside JUN-363. */
export const upstreamProviderRecoveryStore = createUpstreamProviderRecoveryStore();
