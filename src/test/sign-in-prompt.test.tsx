import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SignInPrompt } from "../components/account/SignInPrompt";

const mocks = vi.hoisted(() => ({
  osAccountsLogin: vi.fn(),
  osAccountsCancelLogin: vi.fn(),
}));

vi.mock("../lib/tauri", () => ({
  osAccountsLogin: mocks.osAccountsLogin,
  osAccountsCancelLogin: mocks.osAccountsCancelLogin,
}));

describe("SignInPrompt", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("shows record copy when invoked from the record flow", () => {
    render(
      <SignInPrompt
        open
        reason="record"
        onClose={() => undefined}
        onSignedIn={() => undefined}
      />,
    );
    expect(
      screen.getByRole("heading", { name: "Sign in to record" }),
    ).toBeInTheDocument();
  });

  it("shows dictate copy when invoked from the dictation flow", () => {
    render(
      <SignInPrompt
        open
        reason="dictate"
        onClose={() => undefined}
        onSignedIn={() => undefined}
      />,
    );
    expect(
      screen.getByRole("heading", { name: "Sign in to dictate" }),
    ).toBeInTheDocument();
  });

  it("calls osAccountsLogin and reports the signed-in account", async () => {
    const onSignedIn = vi.fn();
    const account = {
      signedIn: true,
      configured: true,
      user: { id: "usr_1", handle: "jakub" },
    };
    mocks.osAccountsLogin.mockResolvedValue(account);

    render(
      <SignInPrompt
        open
        reason="record"
        onClose={() => undefined}
        onSignedIn={onSignedIn}
      />,
    );
    await userEvent.click(
      screen.getByRole("button", { name: /Sign in with Open Software/i }),
    );
    expect(mocks.osAccountsLogin).toHaveBeenCalledOnce();
    expect(onSignedIn).toHaveBeenCalledWith(account);
  });

  it("shows a busy indicator while sign-in is in flight and surfaces Cancel", async () => {
    let resolveLogin: (value: { signedIn: boolean; configured: boolean }) => void = () => {
      // assigned in the Promise constructor below before any user interaction
    };
    mocks.osAccountsLogin.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveLogin = resolve;
        }),
    );

    render(
      <SignInPrompt
        open
        reason="record"
        onClose={() => undefined}
        onSignedIn={() => undefined}
      />,
    );
    await userEvent.click(
      screen.getByRole("button", { name: /Sign in with Open Software/i }),
    );

    expect(
      screen.getByText(/Waiting for sign-in to complete in your browser/i),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Cancel/i })).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /Sign in with Open Software/i }),
    ).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: /Cancel/i }));
    expect(mocks.osAccountsCancelLogin).toHaveBeenCalledOnce();

    // Resolve the original login promise so the test doesn't hang on cleanup.
    resolveLogin({ signedIn: false, configured: true });
  });

  it("dismisses via Not now without contacting OS Accounts", async () => {
    const onClose = vi.fn();
    render(
      <SignInPrompt
        open
        reason="record"
        onClose={onClose}
        onSignedIn={() => undefined}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: /Not now/i }));
    expect(onClose).toHaveBeenCalledOnce();
    expect(mocks.osAccountsLogin).not.toHaveBeenCalled();
    expect(mocks.osAccountsCancelLogin).not.toHaveBeenCalled();
  });
});
