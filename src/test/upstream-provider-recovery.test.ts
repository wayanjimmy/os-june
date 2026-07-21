import { describe, expect, it, vi } from "vitest";
import type { AgentChatTurn } from "../lib/agent-chat-runtime";
import {
  createUpstreamProviderRecoveryStore,
  upstreamProviderRecoveryIds,
} from "../lib/upstream-provider-recovery";

function providerFailure(id: string): AgentChatTurn {
  return {
    id,
    role: "assistant",
    createdAt: "2026-07-21T08:00:00.000Z",
    status: "complete",
    parts: [
      {
        type: "notice",
        kind: "upstream-provider",
        text: "The model service is temporarily unavailable. Your answer is saved.",
      },
    ],
  };
}

describe("upstream-provider recovery", () => {
  it("uses failure order rather than surface-local turn ids", () => {
    const workspaceIds = upstreamProviderRecoveryIds([
      providerFailure("assistant:workspace-time:1"),
      providerFailure("assistant:workspace-time:2"),
    ]);
    const noteChatIds = upstreamProviderRecoveryIds([
      providerFailure("assistant:note-chat-time:1"),
      providerFailure("assistant:note-chat-time:2"),
    ]);

    expect([...workspaceIds.values()]).toEqual(["upstream-provider:1", "upstream-provider:2"]);
    expect([...noteChatIds.values()]).toEqual([...workspaceIds.values()]);
  });

  it("reserves one process-local attempt and can release a rejected submit", () => {
    const store = createUpstreamProviderRecoveryStore();
    const listener = vi.fn();
    store.subscribe(listener);

    expect(store.reserve("stored-session", "upstream-provider:1")).toBe(true);
    expect(store.reserve("stored-session", "upstream-provider:1")).toBe(false);
    expect(store.attempted("stored-session", "upstream-provider:1")).toBe(true);
    expect(listener).toHaveBeenCalledOnce();

    store.release("stored-session", "upstream-provider:1");
    expect(store.attempted("stored-session", "upstream-provider:1")).toBe(false);
    expect(store.reserve("stored-session", "upstream-provider:1")).toBe(true);
    expect(listener).toHaveBeenCalledTimes(3);
  });
});
