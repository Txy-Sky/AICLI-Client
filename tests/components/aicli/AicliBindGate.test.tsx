import { fireEvent, screen, waitFor } from "@testing-library/react";
import i18n from "i18next";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import tr from "@/i18n/locales/tr.json";
import zh from "@/i18n/locales/zh.json";
import zhTW from "@/i18n/locales/zh-TW.json";
import {
  AicliBindGate,
  useOpenAicliBind,
} from "@/components/aicli/AicliBindGate";
import type { AicliAccountView } from "@/types/aicli";
import { renderWithQueryClient } from "../../utils/testQueryClient";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
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

type Responses = Partial<Record<string, unknown | (() => unknown)>>;

function respond(responses: Responses) {
  invokeMock.mockImplementation(async (command: string) => {
    const reply = responses[command];
    if (reply instanceof Error) throw reply;
    return typeof reply === "function" ? (reply as () => unknown)() : reply;
  });
}

const never = () => new Promise(() => undefined);

function MainApp() {
  const openBind = useOpenAicliBind();
  return (
    <div data-testid="main-app">
      <button type="button" onClick={openBind}>
        open-bind
      </button>
    </div>
  );
}

const renderGate = (bindRequest = 0) =>
  renderWithQueryClient(
    <AicliBindGate bindRequest={bindRequest}>
      <MainApp />
    </AicliBindGate>,
  );

beforeEach(() => {
  invokeMock.mockReset();
});

// F-07：原 F-03 首页未绑定的 4 条用例连同断言迁到这里（绑定界面改由 Gate 的对话框承载）
describe("AicliBindGate — not bound (section 0.4 copy)", () => {
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
    respond({ aicli_get_account: unbound });
    renderGate();

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
    // 未绑定时不进入应用，也不查账户快照与线路
    expect(screen.queryByTestId("main-app")).not.toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith("aicli_refresh", {
      kind: "account",
    });
  });

  it("rejects an sk- key without binding and clears the field", async () => {
    respond({ aicli_get_account: unbound });
    renderGate();
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
    respond({ aicli_get_account: unbound, aicli_bind: never });
    renderGate();
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
    respond({ aicli_get_account: unbound });
    renderGate();
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
      // flowTools（「你的 AI 工具」）在简体和繁体里写法相同，不算漏翻
      const sameAsZh =
        ["failed", "binding"].includes(key) ||
        (lng === "zh-TW" && key === "flowTools");
      if (!sameAsZh) {
        expect(bind[key], `${lng} aicli.bind.${key}`).not.toBe(
          zh.aicli.bind[key],
        );
      }
    }
  });
});

describe("AicliBindGate — gating", () => {
  it("does not enter the app while not bound and Escape cannot close it", async () => {
    respond({ aicli_get_account: unbound });
    renderGate();
    const dialog = await screen.findByRole("dialog", {
      name: "aicli.bind.gateTitle",
    });
    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(
      screen.getByRole("dialog", { name: "aicli.bind.gateTitle" }),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("main-app")).not.toBeInTheDocument();
    expect(screen.queryByText("aicli.bind.later")).not.toBeInTheDocument();
  });

  it("enters the app once the account is bound", async () => {
    let account: AicliAccountView = unbound;
    respond({
      aicli_get_account: () => account,
      aicli_bind: () => {
        account = bound;
        return { state: "bound" };
      },
    });
    renderGate();
    const input = await screen.findByPlaceholderText("aicli.bind.placeholder");
    fireEvent.change(input, { target: { value: "access-value" } });
    fireEvent.click(screen.getByRole("button", { name: "aicli.bind.submit" }));
    expect(await screen.findByTestId("main-app")).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("shows only a backdrop while the binding state loads", async () => {
    respond({ aicli_get_account: never });
    renderGate();
    expect(
      await screen.findByTestId("aicli-bind-gate-loading"),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("main-app")).not.toBeInTheDocument();
  });

  it("does not lock the user out when the binding state cannot be read", async () => {
    respond({ aicli_get_account: new Error("db locked") });
    renderGate();
    expect(await screen.findByTestId("main-app")).toBeInTheDocument();
    expect(
      await screen.findByText("aicli.bind.loadFailed"),
    ).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("enters the app when bound without any dialog", async () => {
    respond({ aicli_get_account: bound });
    renderGate();
    expect(await screen.findByTestId("main-app")).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("prompts once for a new token when it is invalid, and the prompt can be dismissed", async () => {
    respond({ aicli_get_account: { ...bound, state: "needs_rebind" } });
    renderGate();
    expect(await screen.findByTestId("main-app")).toBeInTheDocument();
    const dialog = await screen.findByRole("dialog", {
      name: "aicli.bind.rebindTitle",
    });
    expect(dialog).toHaveTextContent("aicli.account.tokenInvalid");
    fireEvent.click(screen.getByRole("button", { name: "aicli.bind.later" }));
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
    // 首页的「重新绑定」按钮可再次打开
    fireEvent.click(screen.getByRole("button", { name: "open-bind" }));
    expect(
      await screen.findByRole("dialog", { name: "aicli.bind.rebindTitle" }),
    ).toBeInTheDocument();
  });

  it("opens a rebind dialog when the bind deep link arrives while bound", async () => {
    respond({ aicli_get_account: bound });
    const { rerender } = renderGate(0);
    await screen.findByTestId("main-app");
    rerender(
      <AicliBindGate bindRequest={1}>
        <MainApp />
      </AicliBindGate>,
    );
    expect(
      await screen.findByRole("dialog", { name: "aicli.bind.rebindTitle" }),
    ).toBeInTheDocument();
  });
});
