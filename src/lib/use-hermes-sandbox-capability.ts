import { useSyncExternalStore } from "react";
import { sandboxModeSupportStore } from "./hermes-sandbox-capability-store";

export function useSandboxModeSupported(): boolean | undefined {
  return useSyncExternalStore(
    sandboxModeSupportStore.subscribe,
    sandboxModeSupportStore.getSnapshot,
    sandboxModeSupportStore.getSnapshot,
  );
}
