import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AicliHome } from "@/components/aicli/AicliHome";
import type {
  AicliAccountView,
  AicliCachedValue,
  AicliLinesView,
  AicliPrivacyView,
  AicliRoutesView,
} from "@/types/aicli";
import { renderWithQueryClient } from "../../utils/testQueryClient";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

// 本地统计就是原来的用量页（F-03 不改它）：这里只看它拿到的来源标注
vi.mock("@/components/usage/UsagePage", () => ({
  UsagePage: ({ sourceNote }: { sourceNote?: string }) => (
    <div data-testid="usage-page">{sourceNote}</div>
  ),
}));

const unbound: AicliAccountView = {
  state: "not_bound",
  user_id: null,
  username: null,
  user_group: null,
  selected_line_id: "main",
  active_profile_id: null,
  managed_tokens: [],
  quota_display: null,
};

const bound: AicliAccountView = {
  ...unbound,
  state: "bound",
  user_id: 7,
  username: "alice",
  user_group: "vip",
  active_profile_id: "recommended",
};

const cachedAccount = (
  patch: Partial<AicliCachedValue> = {},
): AicliCachedValue => ({
  kind: "account",
  value: { data: { quota: 1_000_000, used_quota: 500_000 } },
  fetched_at: 1_762_560_000,
  stale: false,
  last_error: null,
  ...patch,
});

const lines: AicliLinesView = {
  selected_line_id: "main",
  lines: [
    {
      id: "main",
      label_key: "aicli.lines.main",
      transport: "cloudflare",
      priority: 100,
      first_byte_s: 100,
      idle_s: 110,
      stream_total_s: 600,
      non_stream_s: 110,
      health: {
        checked_at: 1,
        status: "ok",
        latency_ms: 87,
        http_status: 200,
        error_code: null,
        next_check_at: null,
      },
    },
    {
      id: "global",
      label_key: "aicli.lines.global",
      transport: "cloudflare",
      priority: 90,
      first_byte_s: 100,
      idle_s: 110,
      stream_total_s: 600,
      non_stream_s: 110,
      health: null,
    },
  ],
};

const routes = (needsRepair = false): AicliRoutesView => ({
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
        { public_model: "m1", upstream_model: "m1", ordered_groups: ["vip"] },
        { public_model: "m2", upstream_model: "m2", ordered_groups: ["vip"] },
      ],
      repair: {
        conservative: false,
        needs_repair: needsRepair,
        missing_groups: needsRepair ? ["vip"] : [],
        affected_models: needsRepair ? ["m1"] : [],
      },
    },
  ],
});

const privacyOff: AicliPrivacyView = {
  bound: true,
  privacy_enabled: false,
  privacy_failure_mode: "block",
  allow_unscanned_media: false,
  privacy_detect_cjk_names: false,
  privacy_detect_cjk_names_available: false,
  engine: null,
  custom_words: [],
  custom_rules: [],
  secret_backend: null,
};

type Responses = Partial<Record<string, unknown | (() => unknown)>>;

function respond(responses: Responses) {
  invokeMock.mockImplementation(async (command: string) => {
    const reply = responses[command];
    return typeof reply === "function" ? (reply as () => unknown)() : reply;
  });
}

function boundResponses(patch: Responses = {}): Responses {
  return {
    aicli_get_account: bound,
    aicli_refresh: [cachedAccount()],
    aicli_list_lines: lines,
    aicli_get_routes: routes(),
    aicli_get_privacy: privacyOff,
    ...patch,
  };
}

const never = () => new Promise(() => undefined);

beforeEach(() => {
  invokeMock.mockReset();
});

describe("AicliHome — bound", () => {
  it("shows account, line and profile with server-side quota", async () => {
    respond(boundResponses());
    renderWithQueryClient(<AicliHome />);

    expect(await screen.findByText("alice")).toBeInTheDocument();
    expect(screen.getByText("vip")).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByTestId("aicli-quota-remaining")).toHaveTextContent(
        "$2",
      ),
    );
    expect(screen.getByTestId("aicli-quota-used")).toHaveTextContent("$1");
    expect(screen.getByText("aicli.account.source")).toBeInTheDocument();
    expect(
      within(screen.getByTestId("aicli-line-main")).getByText(
        "aicli.lines.current",
      ),
    ).toBeInTheDocument();
    expect(
      await screen.findByText("aicli.profiles.presets.recommended.name"),
    ).toBeInTheDocument();
    expect(screen.queryByText("aicli.account.cached")).not.toBeInTheDocument();
    // 本地统计标注来源（原未绑定用例里的断言，F-07 起首页只在已绑定时出现）
    expect(screen.getByTestId("usage-page")).toHaveTextContent(
      "aicli.local.source",
    );
  });

  it("renders the cached snapshot without waiting for other requests", async () => {
    respond(
      boundResponses({
        aicli_list_lines: never,
        aicli_get_routes: never,
        aicli_get_privacy: never,
      }),
    );
    renderWithQueryClient(<AicliHome />);
    await waitFor(() =>
      expect(screen.getByTestId("aicli-quota-remaining")).toHaveTextContent(
        "$2",
      ),
    );
    expect(screen.getByText("alice")).toBeInTheDocument();
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
  });

  it("keeps showing the cache with its time when a sync failed", async () => {
    respond(
      boundResponses({
        aicli_refresh: [cachedAccount({ stale: true, last_error: "network" })],
      }),
    );
    renderWithQueryClient(<AicliHome />);
    expect(await screen.findByText("aicli.account.cached")).toBeInTheDocument();
    expect(screen.getByTestId("aicli-quota-remaining")).toHaveTextContent("$2");
    expect(screen.getByText("aicli.account.lastSync")).toBeInTheDocument();
  });

  it("shows no data — never 0 — when a sync failed and nothing is cached", async () => {
    respond(boundResponses({ aicli_refresh: [] }));
    const { container } = renderWithQueryClient(<AicliHome />);
    await screen.findByText("alice");
    await waitFor(() =>
      expect(screen.getByTestId("aicli-quota-remaining")).toHaveTextContent(
        "aicli.account.noData",
      ),
    );
    expect(screen.getByTestId("aicli-quota-used")).toHaveTextContent(
      "aicli.account.noData",
    );
    const home = within(container).getByTestId("aicli-home");
    expect(home.textContent).not.toMatch(/[$¥]\s?0(?![.\d])/);
    expect(screen.getByTestId("aicli-quota-remaining").textContent).not.toBe(
      "0",
    );
    expect(screen.getByText("aicli.account.neverSynced")).toBeInTheDocument();
  });

  it("asks for a new token when it is invalid, keeping the cache visible", async () => {
    respond(
      boundResponses({
        aicli_get_account: { ...bound, state: "needs_rebind" },
      }),
    );
    renderWithQueryClient(<AicliHome />);
    expect(
      await screen.findByText("aicli.account.tokenInvalid"),
    ).toBeInTheDocument();
    // F-07：绑定对话框由 AicliBindGate 承载，首页给「重新绑定」入口
    expect(
      screen.getByRole("button", { name: "aicli.bind.rebindAction" }),
    ).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByTestId("aicli-quota-remaining")).toHaveTextContent(
        "$2",
      ),
    );
  });

  it("warns when the active profile needs repair", async () => {
    respond(boundResponses({ aicli_get_routes: routes(true) }));
    renderWithQueryClient(<AicliHome />);
    expect(
      await screen.findByText("aicli.profile.needsRepair"),
    ).toBeInTheDocument();
  });

  it("shows a red notice when availability-first mode degraded", async () => {
    respond(
      boundResponses({
        aicli_get_privacy: {
          ...privacyOff,
          privacy_enabled: true,
          privacy_failure_mode: "pass",
          engine: {
            masked_values: 3,
            degraded: 2,
            engine_blocked: 0,
            media_passed: 0,
          },
        },
      }),
    );
    renderWithQueryClient(<AicliHome />);
    const notice = await screen.findByText("aicli.privacy.degraded");
    expect(notice.closest(".bg-danger-soft")).not.toBeNull();
    expect(screen.getByText("aicli.privacy.engineRunning")).toBeInTheDocument();
  });

  it("shows the engine state without a red notice when nothing degraded", async () => {
    respond(
      boundResponses({
        aicli_get_privacy: { ...privacyOff, privacy_enabled: true },
      }),
    );
    renderWithQueryClient(<AicliHome />);
    expect(
      await screen.findByText("aicli.privacy.engineStopped"),
    ).toBeInTheDocument();
    expect(
      screen.queryByText("aicli.privacy.degraded"),
    ).not.toBeInTheDocument();
  });

  it("switches line from the selector", async () => {
    respond(boundResponses());
    renderWithQueryClient(<AicliHome />);
    const global = await screen.findByTestId("aicli-line-global");
    fireEvent.click(
      within(global).getByRole("button", { name: "aicli.lines.switch" }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("aicli_select_line", {
        id: "global",
      }),
    );
    expect(screen.getByText("aicli.lines.switchNote")).toBeInTheDocument();
  });
});
