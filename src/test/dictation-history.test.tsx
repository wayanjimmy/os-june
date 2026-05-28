import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DictationHistoryView } from "../components/dictation/DictationHistoryView";

const mocks = vi.hoisted(() => ({
  listDictationHistory: vi.fn(),
  dictationSettings: vi.fn(),
  listen: vi.fn(),
  writeText: vi.fn(),
}));

vi.mock("../lib/tauri", () => ({
  listDictationHistory: mocks.listDictationHistory,
  dictationSettings: mocks.dictationSettings,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.listen,
}));

describe("DictationHistoryView", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.listen.mockResolvedValue(vi.fn());
    mocks.writeText.mockResolvedValue(undefined);
    mocks.dictationSettings.mockResolvedValue({
      settings: {
        pushToTalkShortcut: {
          code: "Fn",
          label: "Fn",
          pressCount: 1,
          modifiers: {
            command: false,
            control: false,
            option: false,
            shift: false,
            function: true,
          },
        },
        toggleShortcut: {
          code: "Space",
          label: "Ctrl+Opt+Space",
          pressCount: 1,
          modifiers: {
            command: false,
            control: true,
            option: true,
            shift: false,
            function: false,
          },
        },
        microphone: {},
        style: "standard",
      },
    });
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: mocks.writeText },
    });
    mocks.listDictationHistory.mockResolvedValue({
      retentionDays: 7,
      items: [
        {
          id: "dictation-1",
          text: "Send the follow up.",
          language: "en",
          provider: "openai",
          createdAt: new Date().toISOString(),
        },
      ],
    });
  });

  it("renders recent dictations and copies with trailing space", async () => {
    const user = userEvent.setup();
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: mocks.writeText },
    });
    render(<DictationHistoryView />);

    await waitFor(() =>
      expect(screen.getByText("Send the follow up.")).toBeInTheDocument(),
    );
    expect(
      screen.getByText(/Hold Fn to dictate, or press Ctrl\+Opt\+Space/),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Copy" }));
    await waitFor(() =>
      expect(mocks.writeText).toHaveBeenCalledWith("Send the follow up. "),
    );
  });
});
