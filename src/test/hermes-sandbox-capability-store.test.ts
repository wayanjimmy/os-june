import { describe, expect, it, vi } from "vitest";
import { createSandboxModeSupportStore } from "../lib/hermes-sandbox-capability-store";

describe("sandbox mode support store", () => {
  it("loads and caches exact boolean capabilities", async () => {
    const loadStatus = vi.fn(async () => ({ running: true, sandboxModeSupported: false }));
    const store = createSandboxModeSupportStore(loadStatus);
    const listener = vi.fn();
    const unsubscribe = store.subscribe(listener);

    await expect(store.load()).resolves.toBe(false);
    await expect(store.load()).resolves.toBe(false);
    expect(loadStatus).toHaveBeenCalledTimes(1);
    expect(listener).toHaveBeenCalledTimes(1);
    unsubscribe();
  });

  it("deduplicates concurrent loads", async () => {
    let resolveStatus: (status: { running: boolean; sandboxModeSupported: boolean }) => void = () =>
      undefined;
    const loadStatus = vi.fn(
      () =>
        new Promise<{ running: boolean; sandboxModeSupported: boolean }>((resolve) => {
          resolveStatus = resolve;
        }),
    );
    const store = createSandboxModeSupportStore(loadStatus);

    const first = store.load();
    const second = store.load();
    expect(loadStatus).toHaveBeenCalledTimes(1);
    resolveStatus({ running: true, sandboxModeSupported: true });

    await expect(Promise.all([first, second])).resolves.toEqual([true, true]);
  });

  it("leaves missing and failed capabilities unresolved and retryable", async () => {
    const loadStatus = vi
      .fn()
      .mockResolvedValueOnce({ running: true })
      .mockRejectedValueOnce(new Error("bridge unavailable"))
      .mockResolvedValueOnce({ running: true, sandboxModeSupported: true });
    const store = createSandboxModeSupportStore(loadStatus);

    await expect(store.load()).resolves.toBeUndefined();
    await expect(store.load()).rejects.toThrow("bridge unavailable");
    await expect(store.load()).resolves.toBe(true);
    expect(loadStatus).toHaveBeenCalledTimes(3);
  });

  it("keeps the first defined capability when a pending load settles later", async () => {
    let resolveStatus: (status: { running: boolean; sandboxModeSupported: boolean }) => void = () =>
      undefined;
    const store = createSandboxModeSupportStore(
      () =>
        new Promise((resolve) => {
          resolveStatus = resolve;
        }),
    );

    const pending = store.load();
    store.seedStatus({ sandboxModeSupported: false });
    resolveStatus({ running: true, sandboxModeSupported: true });

    await expect(pending).resolves.toBe(false);
    expect(store.getSnapshot()).toBe(false);
  });

  it("invalidates a pending load when reset for tests", async () => {
    let resolveStatus: (status: { running: boolean; sandboxModeSupported: boolean }) => void = () =>
      undefined;
    const store = createSandboxModeSupportStore(
      () =>
        new Promise((resolve) => {
          resolveStatus = resolve;
        }),
    );

    const pending = store.load();
    store.resetForTests();
    resolveStatus({ running: true, sandboxModeSupported: true });

    await expect(pending).resolves.toBeUndefined();
    expect(store.getSnapshot()).toBeUndefined();
  });
});
