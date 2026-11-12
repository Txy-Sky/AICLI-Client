import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AicliClientsPage } from "@/components/aicli/clients/AicliClientsPage";
import { DEFAULT_VISIBLE_APPS } from "@/config/appConfig";
import type {
  AicliAccountView,
  AicliProfilesView,
  AicliRoutesView,
  AicliTakeoverStatus,
} from "@/types/aicli";
import type { VisibleApps } from "@/types";
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

const route = (id: string, models: string[]) => ({
  id,
  name: id,
  source: id === "recommended" ? "system" : "user",
  preset_id: null,
  revision: 1,
  routes: models.map((model) => ({
    public_model: model,
    upstream_model: model,
    ordered_groups: ["vip"],
  })),
  repair: {
    conservative: false,
    needs_repair: false,
    missing_groups: [],
    affected_models: [],
  },
});

const routes: AicliRoutesView = {
  fingerprint: "f",
  active_profile_id: "recommended",
  selected_line_id: "main",
  profiles: [route("recommended", ["a", "b", "c"]), route("mine", ["a"])],
};

const profiles: AicliProfilesView = {
  profiles: [],
  presets: [],
  bindings: [{ scope: "app", scope_key: "codex", profile_id: "mine" }],
  active_profile_id: "recommended",
};

function respond(patch: Record<string, unknown> = {}) {
  const responses: Record<string, unknown> = {
    aicli_get_account: bound,
    aicli_takeover_status: [
      { app: "claude", state: "active", taken_over_at: 1 },
      { app: "gemini", state: "taking_over", taken_over_at: null },
    ] as AicliTakeoverStatus[],
    aicli_get_profiles: profiles,
    aicli_get_routes: routes,
    ...patch,
  };
  invokeMock.mockImplementation(async (command: string) => {
    const reply = responses[command];
    if (typeof reply === "function") return (reply as () => unknown)();
    return reply;
  });
}

const renderPage = (
  visibleApps: VisibleApps = DEFAULT_VISIBLE_APPS,
  props: Partial<Parameters<typeof AicliClientsPage>[0]> = {},
) => {
  const onOpenApp = vi.fn();
  const onOpenApps = vi.fn();
  renderWithQueryClient(
    <AicliClientsPage
      visibleApps={visibleApps}
      onOpenApp={onOpenApp}
      onOpenApps={onOpenApps}
      {...props}
    />,
  );
  return { onOpenApp, onOpenApps };
};

beforeEach(() => {
  invokeMock.mockReset();
});

describe("AicliClientsPage", () => {
  it("shows one card per supported client with its takeover state and profile", async () => {
    respond();
    renderPage();
    const claude = await screen.findByTestId("aicli-client-card-claude");
    await waitFor(() => expect(claude).toHaveAttribute("data-state", "active"));
    expect(
      within(claude).getByText("aicli.clients.status.active"),
    ).toBeInTheDocument();
    // 2026-11 改版：行尾是开关（开 = 经 AICLI），已接管的行写明停止后回到接管前的配置
    expect(within(claude).getByRole("switch")).toBeChecked();
    expect(
      within(claude).getByText("aicli.clients.restoreTo"),
    ).toBeInTheDocument();
    expect(screen.getByTestId("aicli-client-card-gemini")).toHaveAttribute(
      "data-state",
      "pending",
    );
    expect(screen.getByTestId("aicli-client-card-codex")).toHaveAttribute(
      "data-state",
      "inactive",
    );
    const codexSwitch = within(
      screen.getByTestId("aicli-client-card-codex"),
    ).getByRole("switch");
    expect(codexSwitch).toBeEnabled();
    expect(codexSwitch).not.toBeChecked();
    // 处理中的那一行开关不可点
    expect(
      within(screen.getByTestId("aicli-client-card-gemini")).getByRole(
        "switch",
      ),
    ).toBeDisabled();
    expect(
      screen
        .getAllByTestId(/^aicli-client-card-/)
        .map((card) => card.getAttribute("data-testid")),
    ).toEqual([
      "aicli-client-card-claude",
      "aicli-client-card-claude-desktop",
      "aicli-client-card-codex",
      "aicli-client-card-gemini",
      "aicli-client-card-grokbuild",
      "aicli-client-card-opencode",
    ]);
    // 已接管 1 / 6
    expect(screen.getByText("aicli.clients.summary")).toBeInTheDocument();
  });

  it("takes a client over only after the confirmation dialog", async () => {
    respond({ aicli_take_over: undefined });
    renderPage();
    const codex = await screen.findByTestId("aicli-client-card-codex");
    const takeOver = within(codex).getByRole("switch");
    // 账户读完才可操作
    await waitFor(() => expect(takeOver).toBeEnabled());
    fireEvent.click(takeOver);
    const dialog = await screen.findByRole("dialog");
    expect(
      within(dialog).getByText("aicli.client.takeOverPoint3"),
    ).toBeInTheDocument();
    expect(
      invokeMock.mock.calls.some(([command]) => command === "aicli_take_over"),
    ).toBe(false);
    fireEvent.click(
      within(dialog).getByRole("button", { name: "aicli.client.takeOver" }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("aicli_take_over", {
        app: "codex",
        confirmed: false,
      }),
    );
  });

  it("explains that stopping restores the original configuration", async () => {
    respond({ aicli_release: undefined });
    renderPage();
    expect(
      await screen.findByText("aicli.clients.restoreHint"),
    ).toBeInTheDocument();
    const claude = await screen.findByTestId("aicli-client-card-claude");
    await waitFor(() => expect(claude).toHaveAttribute("data-state", "active"));
    const release = within(claude).getByRole("switch");
    await waitFor(() => expect(release).toBeEnabled());
    fireEvent.click(release);
    const dialog = await screen.findByRole("dialog");
    expect(
      within(dialog).getByText("aicli.client.releaseAfter"),
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

  it("opens the client page from the card and dedicated pages from their row", async () => {
    respond();
    const { onOpenApp, onOpenApps } = renderPage();
    const claude = await screen.findByTestId("aicli-client-card-claude");
    fireEvent.click(
      within(claude).getByRole("button", { name: "aicli.clients.open" }),
    );
    expect(onOpenApp).toHaveBeenCalledWith("claude");
    fireEvent.click(screen.getByTestId("aicli-client-other-openclaw"));
    expect(onOpenApp).toHaveBeenCalledWith("openclaw");
    fireEvent.click(
      screen.getByRole("button", { name: "aicli.clients.installApps" }),
    );
    expect(onOpenApps).toHaveBeenCalled();
  });

  it("lists unsupported clients as coming soon without making them clickable", async () => {
    respond();
    renderPage();
    const soon = await screen.findByTestId("aicli-clients-coming-soon");
    const names = within(soon)
      .getAllByRole("listitem")
      .map((item) => item.textContent);
    // 图标 SVG 自带 <title>，textContent 里会再出现一次名字
    expect(names.some((name) => name?.includes("Kimi Code"))).toBe(true);
    // OpenCode 已可接管（J-02），不再列在这里
    expect(names.some((name) => name?.includes("OpenCode"))).toBe(false);
    expect(within(soon).queryByRole("button")).not.toBeInTheDocument();
    expect(
      screen.queryByTestId("aicli-client-card-kimi"),
    ).not.toBeInTheDocument();
  });

  it("respects hidden clients", async () => {
    respond();
    renderPage({ ...DEFAULT_VISIBLE_APPS, codex: false, openclaw: false });
    await screen.findByTestId("aicli-client-card-claude");
    expect(
      screen.queryByTestId("aicli-client-card-codex"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByTestId("aicli-client-other-openclaw"),
    ).not.toBeInTheDocument();
  });

  it("takes over the remaining clients in one go after a single confirmation", async () => {
    respond({ aicli_take_over: undefined });
    renderPage();
    const rest = await screen.findByTestId("aicli-clients-takeover-rest");
    await waitFor(() => expect(rest).toBeEnabled());
    fireEvent.click(rest);
    const dialog = await screen.findByRole("dialog");
    expect(
      within(dialog).getByText("aicli.clients.bulkBody"),
    ).toBeInTheDocument();
    expect(
      invokeMock.mock.calls.some(([command]) => command === "aicli_take_over"),
    ).toBe(false);
    fireEvent.click(
      within(dialog).getByRole("button", { name: "aicli.client.takeOver" }),
    );
    // 已接管的 claude 和处理中的 gemini 不在其中
    await waitFor(() =>
      expect(
        invokeMock.mock.calls
          .filter(([command]) => command === "aicli_take_over")
          .map(([, args]) => (args as { app: string }).app),
      ).toEqual(["claude-desktop", "codex", "grokbuild", "opencode"]),
    );
    expect(invokeMock).toHaveBeenCalledWith("aicli_take_over", {
      app: "codex",
      confirmed: false,
    });
  });

  it("reports clients the bulk takeover could not take over", async () => {
    respond({
      aicli_take_over: () => {
        throw new Error("boom");
      },
    });
    renderPage();
    const rest = await screen.findByTestId("aicli-clients-takeover-rest");
    await waitFor(() => expect(rest).toBeEnabled());
    fireEvent.click(rest);
    const dialog = await screen.findByRole("dialog");
    fireEvent.click(
      within(dialog).getByRole("button", { name: "aicli.client.takeOver" }),
    );
    expect(
      await screen.findByText("aicli.clients.bulkFailed"),
    ).toBeInTheDocument();
  });

  it("warns when taken-over clients cannot reach the stopped local proxy", async () => {
    respond({ get_proxy_status: { running: false } });
    renderPage();
    expect(
      await screen.findByText("aicli.clients.proxyDown"),
    ).toBeInTheDocument();
  });
});
