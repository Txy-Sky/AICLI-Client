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
    expect(
      within(claude).getByRole("button", { name: "aicli.client.release" }),
    ).toBeInTheDocument();
    expect(screen.getByTestId("aicli-client-card-gemini")).toHaveAttribute(
      "data-state",
      "pending",
    );
    expect(screen.getByTestId("aicli-client-card-codex")).toHaveAttribute(
      "data-state",
      "inactive",
    );
    expect(
      within(screen.getByTestId("aicli-client-card-codex")).getByRole(
        "button",
        { name: "aicli.client.takeOver" },
      ),
    ).toBeEnabled();
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
    ]);
    // 已接管 1 / 5
    expect(screen.getByText("aicli.clients.summary")).toBeInTheDocument();
  });

  it("takes a client over only after the confirmation dialog", async () => {
    respond({ aicli_take_over: undefined });
    renderPage();
    const codex = await screen.findByTestId("aicli-client-card-codex");
    const takeOver = within(codex).getByRole("button", {
      name: "aicli.client.takeOver",
    });
    // 账户读完才可操作
    await waitFor(() => expect(takeOver).toBeEnabled());
    fireEvent.click(takeOver);
    const dialog = await screen.findByRole("dialog");
    expect(
      within(dialog).getByText("aicli.client.confirmTakeOverBody"),
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
    const release = within(claude).getByRole("button", {
      name: "aicli.client.release",
    });
    await waitFor(() => expect(release).toBeEnabled());
    fireEvent.click(release);
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
    expect(names.some((name) => name?.includes("OpenCode"))).toBe(true);
    expect(names.some((name) => name?.includes("Kimi Code"))).toBe(true);
    expect(within(soon).queryByRole("button")).not.toBeInTheDocument();
    expect(
      screen.queryByTestId("aicli-client-card-opencode"),
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

  it("warns when taken-over clients cannot reach the stopped local proxy", async () => {
    respond({ get_proxy_status: { running: false } });
    renderPage();
    expect(
      await screen.findByText("aicli.clients.proxyDown"),
    ).toBeInTheDocument();
  });
});
