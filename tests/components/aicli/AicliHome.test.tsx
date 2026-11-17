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

// 本地统计就是原来的用量页：这里只看它拿到的来源标注，以及 F-10 放进它滚动区的账户面板
vi.mock("@/components/usage/UsagePage", () => ({
  UsagePage: ({
    sourceNote,
    leading,
  }: {
    sourceNote?: string;
    leading?: React.ReactNode;
  }) => (
    <div data-testid="usage-page">
      <span data-testid="usage-source-note">{sourceNote}</span>
      {leading}
    </div>
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
    // F-10：分组名也出现在配置集的分组胶囊里，这里看账户头部的用户组
    expect(
      within(screen.getByTestId("aicli-account-header")).getByText("vip"),
    ).toBeInTheDocument();
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
    // 2026-11 改版：本机统计是单独的「用量」页，不再接在首页下面
    expect(screen.queryByTestId("usage-page")).not.toBeInTheDocument();
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

  it("puts the rebind alert first and marks the account step", async () => {
    respond(
      boundResponses({
        aicli_get_account: { ...bound, state: "needs_rebind" },
        aicli_takeover_status: [
          { app: "claude", state: "active", taken_over_at: 1 },
        ],
        get_proxy_status: { running: false },
      }),
    );
    renderWithQueryClient(<AicliHome />);
    expect(
      await screen.findByText("aicli.overview.alertRebindBody"),
    ).toBeInTheDocument();
    // 一次只显示最严重的一条：代理停了的提示被令牌失效压住
    expect(
      screen.queryByText("aicli.overview.alertProxyTitle"),
    ).not.toBeInTheDocument();
    expect(screen.getByTestId("aicli-flow-account")).toHaveAttribute(
      "data-tone",
      "danger",
    );
  });

  it("offers to start the local proxy when taken-over clients lost it", async () => {
    respond(
      boundResponses({
        aicli_takeover_status: [
          { app: "claude", state: "active", taken_over_at: 1 },
        ],
        get_proxy_status: { running: false },
        start_proxy_server: { address: "127.0.0.1", port: 15821 },
      }),
    );
    renderWithQueryClient(<AicliHome />);
    expect(
      await screen.findByText("aicli.overview.alertProxyTitle"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("aicli.overview.alertProxyBody"),
    ).toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", { name: "aicli.overview.alertProxyAction" }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("start_proxy_server"),
    );
  });

  it("offers the fastest working line when the current one is unreachable", async () => {
    const down = {
      ...lines,
      lines: [
        {
          ...lines.lines[0],
          health: {
            ...lines.lines[0].health!,
            status: "error",
            latency_ms: null,
          },
        },
        {
          ...lines.lines[1],
          health: { ...lines.lines[0].health!, latency_ms: 120 },
        },
      ],
    };
    respond(boundResponses({ aicli_list_lines: down }));
    renderWithQueryClient(<AicliHome />);
    expect(
      await screen.findByText("aicli.overview.alertLineTitle"),
    ).toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", { name: "aicli.overview.alertLineSwitch" }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("aicli_select_line", {
        id: "global",
      }),
    );
  });

  it("asks for a speed test when no other line is known to work", async () => {
    const down = {
      ...lines,
      lines: [
        {
          ...lines.lines[0],
          health: {
            ...lines.lines[0].health!,
            status: "error",
            latency_ms: null,
          },
        },
        lines.lines[1],
      ],
    };
    respond(boundResponses({ aicli_list_lines: down }));
    renderWithQueryClient(<AicliHome />);
    expect(
      await screen.findByText("aicli.overview.alertLineNoFallback"),
    ).toBeInTheDocument();
  });

  it("says the numbers are from the cache and retries the sync", async () => {
    respond(
      boundResponses({
        aicli_refresh: [cachedAccount({ stale: true, last_error: "network" })],
      }),
    );
    renderWithQueryClient(<AicliHome />);
    expect(
      await screen.findByText("aicli.overview.alertStaleTitle"),
    ).toBeInTheDocument();
    expect(screen.getByTestId("aicli-flow-account")).toHaveAttribute(
      "data-tone",
      "stale",
    );
    fireEvent.click(
      screen.getByRole("button", { name: "aicli.overview.alertStaleAction" }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("aicli_refresh", {
        kind: "all",
      }),
    );
  });

  it("shows no alert when everything is fine", async () => {
    respond(boundResponses());
    renderWithQueryClient(<AicliHome />);
    await screen.findByText("alice");
    expect(
      screen.queryByText(/aicli\.overview\.alert/),
    ).not.toBeInTheDocument();
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

  // F-10：服务端每日用量（近 7 天）与额度使用条
  it("shows the last 7 days of server spend and requests with a quota meter", async () => {
    const today = new Date().toISOString().slice(0, 10);
    respond(
      boundResponses({
        aicli_get_usage_summary: {
          days: [
            {
              day: today,
              quota: 250_000,
              requests: 12,
              is_final: false,
              fetched_at: 1,
              stale: false,
            },
            {
              day: "2001-01-01",
              quota: 9_999_999,
              requests: 999,
              is_final: true,
              fetched_at: 1,
              stale: false,
            },
          ],
          total_quota: 10_249_999,
          total_requests: 1011,
        },
      }),
    );
    renderWithQueryClient(<AicliHome />);
    await waitFor(() =>
      expect(screen.getByTestId("aicli-server-spend")).toHaveTextContent(
        "$0.5",
      ),
    );
    // 只算最近 7 天，不把 2001 年那一天算进去
    expect(screen.getByTestId("aicli-server-requests")).toHaveTextContent("12");
    expect(invokeMock).toHaveBeenCalledWith("aicli_get_usage_summary", {
      days: 30,
    });
    // 已用 500000 / (1000000 + 500000) ≈ 33%
    expect(screen.getByRole("meter")).toHaveAttribute("aria-valuenow", "33");
    expect(screen.getByTestId(`mini-bar-${today}`)).not.toHaveAttribute(
      "data-empty",
    );
  });

  it("marks days without server data as gaps instead of zero", async () => {
    respond(boundResponses({ aicli_get_usage_summary: undefined }));
    renderWithQueryClient(<AicliHome />);
    await screen.findByText("alice");
    expect(screen.getByTestId("aicli-server-spend")).toHaveTextContent(
      "aicli.account.noData",
    );
    const today = new Date().toISOString().slice(0, 10);
    expect(screen.getByTestId(`mini-bar-${today}`)).toHaveAttribute(
      "data-empty",
      "true",
    );
    // 没有消耗数据就不估算还能用几天
    expect(screen.queryByTestId("aicli-quota-runway")).not.toBeInTheDocument();
  });

  // 2026-11 改版：请求链路默认展开线路，点别的环节在原地换成那一环的内容
  it("opens another step of the request flow in place", async () => {
    respond(boundResponses());
    renderWithQueryClient(<AicliHome />);
    await screen.findByTestId("aicli-line-main");
    expect(screen.getByTestId("aicli-flow-line")).toHaveAttribute(
      "aria-pressed",
      "true",
    );

    fireEvent.click(screen.getByTestId("aicli-flow-privacy"));
    expect(
      screen.getByText("aicli.overview.flowPrivacyOffDetail"),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("aicli-line-main")).not.toBeInTheDocument();
    expect(screen.getByTestId("aicli-flow-privacy")).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });

  it("estimates how long the balance lasts from recent spend", async () => {
    const today = new Date().toISOString().slice(0, 10);
    respond(
      boundResponses({
        aicli_get_usage_summary: {
          days: [
            {
              day: today,
              quota: 250_000,
              requests: 12,
              is_final: false,
              fetched_at: 1,
              stale: false,
            },
          ],
          total_quota: 250_000,
          total_requests: 12,
        },
      }),
    );
    renderWithQueryClient(<AicliHome />);
    expect(await screen.findByTestId("aicli-quota-runway")).toBeInTheDocument();
  });
});
