import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  AicliClientPage,
  protocolForApp,
  takeoverDisplay,
} from "@/components/aicli/AicliClientPage";
import type {
  AicliAccountView,
  AicliCatalog,
  AicliProfilesView,
  AicliRoutesView,
  AicliTakeoverStatus,
} from "@/types/aicli";
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

const routes: AicliRoutesView = {
  fingerprint: "f",
  active_profile_id: "recommended",
  selected_line_id: "main",
  profiles: [
    {
      id: "recommended",
      // 后端预设的 name 就是 id（profiles.rs::preset），显示名由前端翻译（F-11）
      name: "recommended",
      source: "system",
      preset_id: null,
      revision: 1,
      routes: [
        {
          public_model: "sonnet",
          upstream_model: "up-sonnet",
          ordered_groups: ["vip"],
        },
        {
          public_model: "lite",
          upstream_model: "up-lite",
          ordered_groups: ["vip"],
        },
      ],
      repair: {
        conservative: false,
        needs_repair: false,
        missing_groups: [],
        affected_models: [],
      },
    },
    {
      id: "codex-only",
      name: "Codex 专用",
      source: "user",
      preset_id: null,
      revision: 2,
      routes: [
        {
          public_model: "gpt-x",
          upstream_model: "up-gpt",
          ordered_groups: ["vip"],
        },
      ],
      repair: {
        conservative: false,
        needs_repair: false,
        missing_groups: [],
        affected_models: [],
      },
    },
  ],
};

const catalog: AicliCatalog = {
  groups: { vip: { description: "", ratio: 1, ratio_known: true } },
  auto_available: false,
  auto_groups: [],
  models: {
    "up-sonnet": {
      groups: ["vip"],
      wire: { anthropic: "anthropic" },
      compat: { anthropic: "full", "openai-response": "partial" },
      pricing: { tags: "tools, vision" },
    },
    "up-lite": {
      groups: ["vip"],
      wire: { anthropic: "openai" },
      compat: { anthropic: "partial", "openai-response": "full" },
      pricing: {},
    },
    "up-gpt": {
      groups: ["vip"],
      wire: { "openai-response": "openai-response" },
      compat: { "openai-response": "full" },
      pricing: {},
    },
  },
};

const profiles = (
  bindings: AicliProfilesView["bindings"] = [],
): AicliProfilesView => ({
  profiles: [],
  presets: [],
  bindings,
  active_profile_id: "recommended",
});

type Responses = Record<string, unknown>;

function respond(patch: Responses = {}) {
  const responses: Responses = {
    aicli_get_account: bound,
    aicli_takeover_status: [] as AicliTakeoverStatus[],
    aicli_get_profiles: profiles(),
    aicli_get_routes: routes,
    aicli_get_catalog: catalog,
    ...patch,
  };
  invokeMock.mockImplementation(async (command: string) => {
    const reply = responses[command];
    if (reply instanceof Error) throw reply;
    if (typeof reply === "function") return (reply as () => unknown)();
    return reply;
  });
}

const callsTo = (command: string) =>
  invokeMock.mock.calls.filter(([name]) => name === command);

beforeEach(() => {
  invokeMock.mockReset();
});

describe("helpers", () => {
  it("maps takeover states to what the page shows", () => {
    expect(takeoverDisplay("active")).toBe("active");
    expect(takeoverDisplay("taking_over")).toBe("pending");
    expect(takeoverDisplay("releasing")).toBe("pending");
    expect(takeoverDisplay("released")).toBe("inactive");
    expect(takeoverDisplay(undefined)).toBe("inactive");
  });

  it("uses the client's wire protocol for compatibility", () => {
    expect(protocolForApp("claude")).toBe("anthropic");
    expect(protocolForApp("claude-desktop")).toBe("anthropic");
    expect(protocolForApp("gemini")).toBe("gemini");
    expect(protocolForApp("codex")).toBe("openai-response");
  });
});

describe("AicliClientPage", () => {
  it("asks to bind first while unbound", async () => {
    respond({ aicli_get_account: { ...bound, state: "not_bound" } });
    renderWithQueryClient(<AicliClientPage app="claude" />);
    expect(
      await screen.findByText("aicli.client.notBound"),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "aicli.client.takeOver" }),
    ).not.toBeInTheDocument();
    expect(callsTo("aicli_takeover_status")).toHaveLength(0);
  });

  it("takes over after the confirmation that explains the restore", async () => {
    respond({ aicli_take_over: undefined });
    renderWithQueryClient(<AicliClientPage app="claude" />);
    expect(await screen.findByTestId("aicli-takeover-state")).toHaveTextContent(
      "aicli.client.status.inactive",
    );
    fireEvent.click(
      screen.getByRole("button", { name: "aicli.client.takeOver" }),
    );
    const dialog = await screen.findByRole("dialog");
    expect(
      within(dialog).getByText("aicli.client.confirmTakeOverBody"),
    ).toBeInTheDocument();
    expect(callsTo("aicli_take_over")).toHaveLength(0);
    fireEvent.click(
      within(dialog).getByRole("button", { name: "aicli.client.takeOver" }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("aicli_take_over", {
        app: "claude",
        confirmed: false,
      }),
    );
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
  });

  it("shows the conflict and only overwrites after a second confirmation", async () => {
    let attempts = 0;
    respond({
      aicli_take_over: () => {
        attempts += 1;
        if (attempts === 1) {
          throw { code: "cc_switch_conflict", message: "conflict" };
        }
        return undefined;
      },
    });
    renderWithQueryClient(<AicliClientPage app="codex" />);
    fireEvent.click(
      await screen.findByRole("button", { name: "aicli.client.takeOver" }),
    );
    fireEvent.click(
      within(await screen.findByRole("dialog")).getByRole("button", {
        name: "aicli.client.takeOver",
      }),
    );
    const conflict = await screen.findByRole("dialog", {
      name: "aicli.client.conflictTitle",
    });
    expect(screen.getByTestId("aicli-takeover-state")).toHaveTextContent(
      "aicli.client.status.conflict",
    );
    expect(
      within(conflict).getByText("aicli.client.conflictBody"),
    ).toBeInTheDocument();
    fireEvent.click(
      within(conflict).getByRole("button", {
        name: "aicli.client.overrideAndTakeOver",
      }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("aicli_take_over", {
        app: "codex",
        confirmed: true,
      }),
    );
  });

  it("stops the takeover after a confirmation saying the config is restored", async () => {
    respond({
      aicli_takeover_status: [
        { app: "claude", state: "active", taken_over_at: 1 },
      ],
      aicli_release: undefined,
    });
    renderWithQueryClient(<AicliClientPage app="claude" />);
    expect(await screen.findByTestId("aicli-takeover-state")).toHaveTextContent(
      "aicli.client.status.active",
    );
    fireEvent.click(
      screen.getByRole("button", { name: "aicli.client.release" }),
    );
    const dialog = await screen.findByRole("dialog");
    expect(
      within(dialog).getByText("aicli.client.confirmReleaseBody"),
    ).toBeInTheDocument();
    fireEvent.click(
      within(dialog).getByRole("button", { name: "aicli.client.release" }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("aicli_release", {
        app: "claude",
      }),
    );
  });

  it("disables takeover while a takeover or release is in progress", async () => {
    respond({
      aicli_takeover_status: [
        { app: "claude", state: "taking_over", taken_over_at: null },
      ],
    });
    renderWithQueryClient(<AicliClientPage app="claude" />);
    expect(await screen.findByTestId("aicli-takeover-state")).toHaveTextContent(
      "aicli.client.status.pending",
    );
    expect(
      screen.getByRole("button", { name: "aicli.client.takeOver" }),
    ).toBeDisabled();
  });

  it("does not offer takeover for clients AICLI cannot take over", async () => {
    respond();
    renderWithQueryClient(<AicliClientPage app="opencode" />);
    expect(
      await screen.findByText("aicli.client.unsupported"),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "aicli.client.takeOver" }),
    ).not.toBeInTheDocument();
  });

  it("lists the client's models with profile, line, compatibility and tags", async () => {
    respond();
    renderWithQueryClient(<AicliClientPage app="claude" />);
    const row = await screen.findByTestId("aicli-model-sonnet");
    const cells = within(row).getAllByRole("cell");
    expect(cells[0]).toHaveTextContent("sonnet");
    expect(cells[1]).toHaveTextContent(
      "aicli.profiles.presets.recommended.name",
    );
    expect(cells[2]).toHaveTextContent("aicli.lines.main");
    expect(cells[3]).toHaveTextContent("aicli.client.compat.full");
    expect(within(cells[4]).getByText("tools")).toBeInTheDocument();
    expect(within(cells[4]).getByText("vision")).toBeInTheDocument();
    expect(
      within(screen.getByTestId("aicli-model-lite")).getByText(
        "aicli.client.compat.partial",
      ),
    ).toBeInTheDocument();
  });

  it("warns about partial compatibility on Claude Code", async () => {
    respond();
    renderWithQueryClient(<AicliClientPage app="claude" />);
    expect(
      await screen.findByText("aicli.client.partialNotice"),
    ).toBeInTheDocument();
  });

  it("does not show the Claude Code partial warning for other clients", async () => {
    respond();
    renderWithQueryClient(<AicliClientPage app="codex" />);
    expect(await screen.findByTestId("aicli-model-sonnet")).toBeInTheDocument();
    expect(
      screen.queryByText("aicli.client.partialNotice"),
    ).not.toBeInTheDocument();
  });

  it("uses the client's own profile binding before the default", async () => {
    respond({
      aicli_get_profiles: profiles([
        { scope: "app", scope_key: "codex", profile_id: "codex-only" },
      ]),
    });
    renderWithQueryClient(<AicliClientPage app="codex" />);
    expect(await screen.findByTestId("aicli-model-gpt-x")).toBeInTheDocument();
    expect(screen.queryByTestId("aicli-model-sonnet")).not.toBeInTheDocument();
  });

  it("refreshes the model catalog on request", async () => {
    respond({ aicli_refresh: [] });
    renderWithQueryClient(<AicliClientPage app="claude" />);
    fireEvent.click(
      await screen.findByRole("button", {
        name: "aicli.client.refreshCatalog",
      }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("aicli_refresh", { kind: "all" }),
    );
    await waitFor(() =>
      expect(callsTo("aicli_get_catalog").length).toBeGreaterThan(1),
    );
  });
});
