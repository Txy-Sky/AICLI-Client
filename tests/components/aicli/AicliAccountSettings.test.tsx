import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AicliAccountSettings } from "@/components/aicli/AicliAccountSettings";
import type { AicliAccountView, AicliCachedValue } from "@/types/aicli";
import { renderWithQueryClient } from "../../utils/testQueryClient";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

const bound: AicliAccountView = {
  state: "bound",
  user_id: 7,
  username: "alice",
  user_group: "vip",
  selected_line_id: "main",
  active_profile_id: "recommended",
  managed_tokens: [],
  quota_display: null,
};

const cached: AicliCachedValue = {
  kind: "account",
  value: { data: { quota: 1_000_000, used_quota: 500_000 } },
  fetched_at: 1_762_560_000,
  stale: false,
  last_error: null,
};

function respond(patch: Record<string, unknown> = {}) {
  const responses: Record<string, unknown> = {
    aicli_get_account: bound,
    aicli_refresh: [cached],
    aicli_unbind: undefined,
    ...patch,
  };
  invokeMock.mockImplementation(async (command: string) => {
    const reply = responses[command];
    if (typeof reply === "function") return (reply as () => unknown)();
    return reply;
  });
}

beforeEach(() => {
  invokeMock.mockReset();
});

describe("AicliAccountSettings", () => {
  it("shows the bound account with what is left", async () => {
    respond();
    renderWithQueryClient(<AicliAccountSettings />);
    const block = await screen.findByTestId("aicli-account-settings");
    expect(within(block).getByText("alice")).toBeInTheDocument();
    expect(within(block).getByText("vip")).toBeInTheDocument();
    expect(
      within(block).getByText("aicli.accountSettings.boundHere"),
    ).toBeInTheDocument();
  });

  it("renders nothing while not bound", async () => {
    respond({ aicli_get_account: { ...bound, state: "not_bound" } });
    const { container } = renderWithQueryClient(<AicliAccountSettings />);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("aicli_get_account"),
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("unbinds only after confirming, keeping created tokens by default", async () => {
    respond();
    renderWithQueryClient(<AicliAccountSettings />);
    fireEvent.click(
      await screen.findByRole("button", {
        name: "aicli.accountSettings.unbind",
      }),
    );
    const dialog = await screen.findByRole("dialog");
    expect(invokeMock).not.toHaveBeenCalledWith(
      "aicli_unbind",
      expect.anything(),
    );
    expect(within(dialog).getByRole("checkbox")).not.toBeChecked();
    fireEvent.click(
      within(dialog).getByRole("button", {
        name: "aicli.accountSettings.unbind",
      }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("aicli_unbind", {
        deleteCreatedTokens: false,
      }),
    );
  });

  it("deletes the created tokens when asked to", async () => {
    respond();
    renderWithQueryClient(<AicliAccountSettings />);
    fireEvent.click(
      await screen.findByRole("button", {
        name: "aicli.accountSettings.unbind",
      }),
    );
    const dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("checkbox"));
    fireEvent.click(
      within(dialog).getByRole("button", {
        name: "aicli.accountSettings.unbind",
      }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("aicli_unbind", {
        deleteCreatedTokens: true,
      }),
    );
  });

  it("says so when unbinding fails", async () => {
    respond({
      aicli_unbind: () => {
        throw { code: "restore_failed", message: "restore failed" };
      },
    });
    renderWithQueryClient(<AicliAccountSettings />);
    fireEvent.click(
      await screen.findByRole("button", {
        name: "aicli.accountSettings.unbind",
      }),
    );
    fireEvent.click(
      within(await screen.findByRole("dialog")).getByRole("button", {
        name: "aicli.accountSettings.unbind",
      }),
    );
    expect(
      await screen.findByText("aicli.accountSettings.unbindFailed"),
    ).toBeInTheDocument();
  });
});
