import { hermesBridgeStatus, type HermesBridgeStatus } from "./tauri";

export type SandboxModeSupported = boolean | undefined;

export type SandboxModeSupportStore = {
  getSnapshot(): SandboxModeSupported;
  subscribe(listener: () => void): () => void;
  load(): Promise<SandboxModeSupported>;
  seedStatus(status: Pick<HermesBridgeStatus, "sandboxModeSupported">): void;
  resetForTests(): void;
};

export function createSandboxModeSupportStore(
  loadStatus: () => Promise<HermesBridgeStatus>,
): SandboxModeSupportStore {
  let snapshot: SandboxModeSupported;
  let pendingLoad: Promise<SandboxModeSupported> | undefined;
  let automaticLoadAttempted = false;
  let generation = 0;
  const listeners = new Set<() => void>();

  function publish(value: boolean) {
    if (snapshot !== undefined) return;
    snapshot = value;
    for (const listener of listeners) listener();
  }

  function load(): Promise<SandboxModeSupported> {
    if (snapshot !== undefined) return Promise.resolve(snapshot);
    if (pendingLoad) return pendingLoad;
    const loadGeneration = generation;
    const request = loadStatus().then((status) => {
      if (generation !== loadGeneration) return snapshot;
      if (status.sandboxModeSupported === true || status.sandboxModeSupported === false) {
        publish(status.sandboxModeSupported);
      }
      return snapshot;
    });
    pendingLoad = request;
    const clearPending = () => {
      if (pendingLoad === request) pendingLoad = undefined;
    };
    request.then(clearPending, clearPending);
    return request;
  }

  return {
    getSnapshot() {
      return snapshot;
    },
    subscribe(listener) {
      listeners.add(listener);
      if (snapshot === undefined && !automaticLoadAttempted) {
        automaticLoadAttempted = true;
        void load().catch(() => undefined);
      }
      return () => listeners.delete(listener);
    },
    load,
    seedStatus(status) {
      if (status.sandboxModeSupported === true || status.sandboxModeSupported === false) {
        publish(status.sandboxModeSupported);
      }
    },
    resetForTests() {
      generation += 1;
      pendingLoad = undefined;
      automaticLoadAttempted = false;
      snapshot = undefined;
      for (const listener of listeners) listener();
    },
  };
}

export const sandboxModeSupportStore = createSandboxModeSupportStore(hermesBridgeStatus);

export const getSandboxModeSupported = sandboxModeSupportStore.getSnapshot;
export const loadSandboxModeSupported = sandboxModeSupportStore.load;
export const seedSandboxModeSupported = sandboxModeSupportStore.seedStatus;

export function resetSandboxModeSupportForTests() {
  sandboxModeSupportStore.resetForTests();
}

export function seedSandboxModeSupportedForTests(value: SandboxModeSupported) {
  resetSandboxModeSupportForTests();
  if (value !== undefined) seedSandboxModeSupported({ sandboxModeSupported: value });
}
