import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import i18n from "i18next";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import tr from "@/i18n/locales/tr.json";
import zh from "@/i18n/locales/zh.json";
import zhTW from "@/i18n/locales/zh-TW.json";
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
      name: "推荐",
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

describe("AicliHome — not bound", () => {
  beforeEach(async () => {
    i18n.addResourceBundle(
      "zh",
      "translation",
      { aicli: zh.aicli },
      true,
      true,
    );
    await i18n.changeLanguage("zh");
  });

  afterEach(() => {
    i18n.removeResourceBundle("zh", "translation");
    i18n.addResourceBundle("zh", "translation", {});
  });

  it("shows the bind card with the section 0.4 copy, word for word", async () => {
    respond({ aicli_get_account: unbound, aicli_get_privacy: privacyOff });
    renderWithQueryClient(<AicliHome />);

    expect(
      await screen.findByRole("heading", { name: "系统访问令牌" }),
    ).toBeInTheDocument();
    expect(
      screen.getByPlaceholderText(
        "粘贴系统访问令牌（不是 sk- 开头的 API 密钥）",
      ),
    ).toHaveAttribute("type", "password");
    expect(
      screen.getByText(
        "在网页「个人设置」中生成。令牌只显示一次，请妥善保存；绑定其他设备时需要粘贴同一个令牌。",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "重新生成后旧令牌立即失效，所有已绑定的设备都要粘贴新令牌。",
      ),
    ).toBeInTheDocument();
    expect(screen.getByTestId("usage-page")).toHaveTextContent(
      "来源：本机记录",
    );
    // 未绑定时不查账户快照与线路
    expect(invokeMock).not.toHaveBeenCalledWith("aicli_refresh", {
      kind: "account",
    });
  });

  it("rejects an sk- key without binding and clears the field", async () => {
    respond({ aicli_get_account: unbound, aicli_get_privacy: privacyOff });
    renderWithQueryClient(<AicliHome />);
    const input = await screen.findByPlaceholderText(
      "粘贴系统访问令牌（不是 sk- 开头的 API 密钥）",
    );
    fireEvent.change(input, { target: { value: ["sk-", "abc123"].join("") } });
    fireEvent.click(screen.getByRole("button", { name: "绑定" }));

    expect(
      await screen.findByText(
        "这是 API 密钥，请粘贴网页「个人设置」中的系统访问令牌。",
      ),
    ).toBeInTheDocument();
    expect(input).toHaveValue("");
    expect(
      invokeMock.mock.calls.some(([command]) => command === "aicli_bind"),
    ).toBe(false);
  });

  it("binds with the pasted token and clears the field at once", async () => {
    respond({
      aicli_get_account: unbound,
      aicli_get_privacy: privacyOff,
      aicli_bind: never,
    });
    renderWithQueryClient(<AicliHome />);
    const input = await screen.findByPlaceholderText(
      "粘贴系统访问令牌（不是 sk- 开头的 API 密钥）",
    );
    fireEvent.change(input, { target: { value: "access-value" } });
    fireEvent.click(screen.getByRole("button", { name: "绑定" }));
    expect(input).toHaveValue("");
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("aicli_bind", {
        accessToken: "access-value",
        line: "main",
      }),
    );
  });

  it("opens the profile page in the system browser", async () => {
    respond({ aicli_get_account: unbound, aicli_get_privacy: privacyOff });
    renderWithQueryClient(<AicliHome />);
    fireEvent.click(
      await screen.findByRole("button", { name: "打开网页获取令牌" }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("open_external", {
        url: "https://aicliapi.com/profile",
      }),
    );
  });
});

describe("bind copy translations", () => {
  const bundles = { en, ja, tr, "zh-TW": zhTW } as const;
  const keys = Object.keys(zh.aicli.bind) as Array<keyof typeof zh.aicli.bind>;

  it.each(Object.keys(bundles))("%s translates every bind string", (lng) => {
    const bind = bundles[lng as keyof typeof bundles].aicli.bind as Record<
      string,
      string
    >;
    for (const key of keys) {
      expect(bind[key], `${lng} aicli.bind.${key}`).toBeTruthy();
      if (!["failed", "binding"].includes(key)) {
        expect(bind[key], `${lng} aicli.bind.${key}`).not.toBe(
          zh.aicli.bind[key],
        );
      }
    }
  });
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
    expect(await screen.findByText("推荐")).toBeInTheDocument();
    expect(screen.queryByText("aicli.account.cached")).not.toBeInTheDocument();
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
    expect(
      screen.getByRole("heading", { name: "aicli.bind.title" }),
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

  it("opens a rebind dialog when the bind deep link arrives while bound", async () => {
    respond(boundResponses());
    const { rerender } = renderWithQueryClient(<AicliHome bindRequest={0} />);
    await screen.findByText("alice");
    rerender(<AicliHome bindRequest={1} />);
    expect(
      await screen.findByRole("dialog", { name: "aicli.bind.rebindTitle" }),
    ).toBeInTheDocument();
  });
});
