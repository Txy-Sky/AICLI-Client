import {
  act,
  fireEvent,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AicliWalletPage } from "@/components/aicli/wallet/AicliWalletPage";
import { lastDays } from "@/components/aicli/AicliOverviewPanel";
import type {
  AicliAccountView,
  AicliCachedValue,
  AicliPayStarted,
  AicliPlan,
  AicliTopupHistory,
  AicliTopupInfo,
  AicliWalletView,
} from "@/types/aicli";
import { emitTauriEvent } from "../../msw/tauriMocks";
import { renderWithQueryClient } from "../../utils/testQueryClient";

const invokeMock = vi.fn();
const toastMock = vi.hoisted(() => ({
  success: vi.fn(),
  error: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@/lib/toast", () => ({ toast: toastMock }));

const account: AicliAccountView = {
  state: "bound",
  user_id: 7,
  username: "alice",
  user_group: "vip",
  selected_line_id: "main",
  active_profile_id: null,
  managed_tokens: [],
  quota_display: null,
};

const cachedAccount: AicliCachedValue = {
  kind: "account",
  value: {
    data: {
      quota: 1_000_000,
      used_quota: 500_000,
      aff_quota: 750_000,
      aff_history_quota: 2_000_000,
      aff_count: 3,
    },
  },
  fetched_at: 1_762_560_000,
  stale: false,
  last_error: null,
};

const topup = (patch: Partial<AicliTopupInfo> = {}): AicliTopupInfo => ({
  online_topup: true,
  redemption: true,
  methods: [
    { name: "支付宝", type: "alipay", color: null, min_topup: null },
    { name: "微信", type: "wxpay", color: null, min_topup: 50 },
  ],
  min_topup: 10,
  amount_options: [10, 100],
  discounts: { "100": 0.9 },
  topup_link: null,
  ...patch,
});

const plan = (patch: Partial<AicliPlan> = {}): AicliPlan => ({
  id: 3,
  title: "Pro",
  subtitle: "",
  price_amount: 9.9,
  currency: "USD",
  duration_unit: "month",
  duration_value: 1,
  custom_seconds: 0,
  allow_balance_pay: null,
  max_purchase_per_user: 0,
  upgrade_group: "",
  total_amount: 5_000_000,
  quota_reset_period: "never",
  ...patch,
});

const view = (patch: Partial<AicliWalletView> = {}): AicliWalletView => ({
  topup: topup(),
  plans: [],
  subscriptions: [],
  billing_preference: null,
  aff_code: "abcd",
  failed: [],
  ...patch,
});

const history: AicliTopupHistory = {
  items: [
    {
      id: 1,
      amount: 10,
      money: 7.3,
      trade_no: "USR7NOabc",
      payment_method: "alipay",
      create_time: 1_762_560_000,
      complete_time: 1_762_560_100,
      status: "success",
    },
    {
      id: 2,
      amount: 10,
      money: 10,
      trade_no: "USR7NOold",
      payment_method: "wxpay",
      create_time: 1_762_550_000,
      complete_time: 0,
      status: "pending",
    },
  ],
  total: 2,
};

const started: AicliPayStarted = {
  trade_no: "USR7NOnew",
  host: "pay.example.test",
  completed: false,
};

// 近 7 天共消耗 3.5 单位：日均 0.5，余额 2 单位 → 约可用 4 天
const usageSummary = {
  days: lastDays(7).map((day) => ({
    day,
    quota: 250_000,
    requests: 3,
    is_final: true,
    fetched_at: 1,
    stale: false,
  })),
  total_quota: 1_750_000,
  total_requests: 21,
};

const payButton = () => screen.getByTestId("aicli-wallet-pay-button");

function respond(patch: Record<string, unknown> = {}) {
  const responses: Record<string, unknown> = {
    aicli_get_usage_summary: usageSummary,
    aicli_get_account: account,
    aicli_refresh: [cachedAccount],
    aicli_wallet_get: view(),
    aicli_wallet_history: history,
    aicli_wallet_quote: "7.30",
    ...patch,
  };
  invokeMock.mockImplementation(async (command: string, args: unknown) => {
    const reply = responses[command];
    if (typeof reply === "function") {
      return (reply as (args: unknown) => unknown)(args);
    }
    return reply;
  });
}

const calls = (command: string) =>
  invokeMock.mock.calls.filter(([name]) => name === command);

beforeEach(() => {
  invokeMock.mockReset();
  toastMock.success.mockReset();
  toastMock.error.mockReset();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("AicliWalletPage", () => {
  it("shows the balance, quotes every preset on the server and starts a payment", async () => {
    respond({
      aicli_wallet_pay: started,
      aicli_wallet_order_status: "success",
      aicli_wallet_quote: (args: { amount: number }) =>
        (args.amount * 1).toFixed(2),
    });
    renderWithQueryClient(<AicliWalletPage />);

    expect(await screen.findByTestId("aicli-wallet-topup")).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByTestId("aicli-wallet-balance")).toHaveTextContent("2"),
    );
    expect(await screen.findByTestId("aicli-wallet-runway")).toHaveTextContent(
      "aicli.overview.runway",
    );
    // 兑换码和邀请返利都在左栏
    expect(screen.getByTestId("aicli-wallet-redeem")).toBeInTheDocument();
    // 每一档都向服务端要实付金额；选中档与第一档同一个查询，不重复请求
    await waitFor(() =>
      expect(
        calls("aicli_wallet_quote")
          .map(([, args]) => (args as { amount: number }).amount)
          .sort((a, b) => a - b),
      ).toEqual([10, 100]),
    );
    await waitFor(() =>
      expect(payButton()).toHaveTextContent("aicli.wallet.topup.payWithMoney"),
    );
    expect(screen.getByTestId("aicli-wallet-summary")).toHaveTextContent(
      "aicli.wallet.topup.money",
    );

    const methods = screen.getByRole("group", {
      name: "aicli.wallet.topup.methodLabel",
    });
    fireEvent.click(within(methods).getByRole("button", { name: "微信" }));
    // 微信最少 50：当前 10 不够，不能支付
    expect(payButton()).toBeDisabled();
    expect(screen.getByRole("alert")).toHaveTextContent(
      "aicli.wallet.topup.minHint",
    );
    const custom = screen.getByRole("textbox", {
      name: "aicli.wallet.topup.customLabel",
    });
    fireEvent.change(custom, { target: { value: "120" } });
    // 输入自定义数量后预设不再选中
    for (const preset of screen
      .getByTestId("aicli-wallet-topup")
      .querySelectorAll('button[aria-pressed="true"]')) {
      expect(methods.contains(preset)).toBe(true);
    }
    await waitFor(() => expect(payButton()).toBeEnabled());
    fireEvent.click(payButton());

    expect(
      await screen.findByTestId("aicli-wallet-pay-waiting"),
    ).toHaveTextContent("aicli.wallet.pay.waitingTitle");
    expect(calls("aicli_wallet_pay")).toEqual([
      ["aicli_wallet_pay", { amount: 120, method: "wxpay" }],
    ]);
    // 等待期间不能再下一单
    expect(payButton()).toBeDisabled();
    expect(payButton()).toHaveTextContent("aicli.wallet.topup.paying");

    act(() => {
      emitTauriEvent("aicli-pay-finished", {
        kind: "topup",
        trade_no: "USR7NOnew",
        outcome: "success",
      });
    });
    expect(
      await screen.findByTestId("aicli-wallet-pay-success"),
    ).toHaveTextContent("aicli.wallet.pay.successTopup");
    expect(calls("aicli_wallet_order_status")[0]).toEqual([
      "aicli_wallet_order_status",
      { tradeNo: "USR7NOnew" },
    ]);
  });

  it("stops waiting with a hint when the order is still unpaid after the window closed", async () => {
    respond({
      aicli_wallet_pay: started,
      aicli_wallet_order_status: "pending",
    });
    renderWithQueryClient(<AicliWalletPage />);
    await screen.findByTestId("aicli-wallet-topup");
    await waitFor(() => expect(payButton()).toBeEnabled());
    fireEvent.click(payButton());
    await screen.findByTestId("aicli-wallet-pay-waiting");

    const closed = { kind: "topup", trade_no: "USR7NOnew", outcome: "closed" };
    act(() => emitTauriEvent("aicli-pay-finished", closed));
    await waitFor(() =>
      expect(screen.getByTestId("aicli-wallet-pay-waiting")).toHaveTextContent(
        "aicli.wallet.pay.confirmingTitle",
      ),
    );
    await waitFor(() =>
      expect(calls("aicli_wallet_order_status").length).toBeGreaterThan(0),
    );

    // 关窗后的确认时间过去了，订单仍是待支付
    const later = Date.now() + 60_000;
    vi.spyOn(Date, "now").mockReturnValue(later);
    act(() => emitTauriEvent("aicli-pay-finished", closed));
    expect(
      await screen.findByTestId("aicli-wallet-pay-not-detected"),
    ).toHaveTextContent("aicli.wallet.pay.notDetectedTitle");
  });

  it("counts down, closes the pay window when the order times out and on cancel", async () => {
    respond({
      aicli_wallet_pay: started,
      aicli_wallet_order_status: "pending",
      aicli_wallet_close_pay: true,
    });
    renderWithQueryClient(<AicliWalletPage />);
    await screen.findByTestId("aicli-wallet-topup");
    await waitFor(() => expect(payButton()).toBeEnabled());
    // 支付宝、微信用官方标志，不是文字色块
    const methods = screen.getByRole("group", {
      name: "aicli.wallet.topup.methodLabel",
    });
    expect(methods.querySelector('svg[data-brand="alipay"]')).not.toBeNull();
    expect(methods.querySelector('svg[data-brand="wechat"]')).not.toBeNull();

    fireEvent.click(payButton());
    await screen.findByTestId("aicli-wallet-pay-waiting");
    expect(screen.getByTestId("aicli-wallet-countdown").textContent).toMatch(
      /^(5:00|4:5\d)$/,
    );
    expect(calls("aicli_wallet_close_pay")).toEqual([]);

    // 订单 5 分钟超时：到点关闭支付窗口，转入确认到账
    const later = Date.now() + 5 * 60_000 + 1_000;
    const clock = vi.spyOn(Date, "now").mockReturnValue(later);
    await waitFor(
      () => expect(calls("aicli_wallet_close_pay")).toHaveLength(1),
      { timeout: 4_000 },
    );
    await waitFor(() =>
      expect(screen.getByTestId("aicli-wallet-pay-waiting")).toHaveTextContent(
        "aicli.wallet.pay.confirmingTitle",
      ),
    );
    expect(screen.queryByTestId("aicli-wallet-countdown")).toBeNull();
    // 只恢复时钟：restoreAllMocks 会连 invoke 的调用记录一起清掉
    clock.mockRestore();

    // 「不再等待」后重新下单，再点「取消支付」：窗口一并关掉
    fireEvent.click(
      screen.getByRole("button", { name: "aicli.wallet.pay.stop" }),
    );
    expect(calls("aicli_wallet_close_pay")).toHaveLength(1);
    await waitFor(() => expect(payButton()).toBeEnabled());
    fireEvent.click(payButton());
    fireEvent.click(
      await screen.findByRole("button", { name: "aicli.wallet.pay.cancel" }),
    );
    await waitFor(() =>
      expect(calls("aicli_wallet_close_pay")).toHaveLength(2),
    );
    expect(screen.queryByTestId("aicli-wallet-pay-waiting")).toBeNull();
  });

  it("reports the server's own reason when an order is declined", async () => {
    respond({
      aicli_wallet_pay: () =>
        Promise.reject({
          code: "wallet_declined",
          message: "充值数量不能小于 10",
        }),
    });
    renderWithQueryClient(<AicliWalletPage />);
    await screen.findByTestId("aicli-wallet-topup");
    await waitFor(() => expect(payButton()).toBeEnabled());
    fireEvent.click(payButton());
    await waitFor(() =>
      expect(toastMock.error).toHaveBeenCalledWith("充值数量不能小于 10"),
    );
    expect(screen.queryByTestId("aicli-wallet-pay-waiting")).toBeNull();
  });

  it("hides plans when the operator configured none and falls back to the website without epay", async () => {
    respond({
      aicli_wallet_get: view({
        topup: topup({ online_topup: false, redemption: false, methods: [] }),
        aff_code: null,
      }),
      open_external: true,
    });
    renderWithQueryClient(<AicliWalletPage />);

    expect(
      await screen.findByTestId("aicli-wallet-topup-off"),
    ).toBeInTheDocument();
    for (const absent of [
      "aicli-wallet-topup",
      "aicli-wallet-plans",
      "aicli-wallet-redeem",
      "aicli-wallet-invite",
    ]) {
      expect(screen.queryByTestId(absent)).toBeNull();
    }
    fireEvent.click(
      screen.getByRole("button", { name: "aicli.wallet.topup.openWebsite" }),
    );
    await waitFor(() =>
      expect(calls("open_external")).toEqual([
        ["open_external", { url: "https://aicliapi.com" }],
      ]),
    );
  });

  it("buys a plan from the balance or through epay after choosing how to pay", async () => {
    respond({
      aicli_wallet_get: view({ plans: [plan()] }),
      aicli_subscription_buy: (args: { method: string }) =>
        args.method === "balance"
          ? { trade_no: null, host: null, completed: true }
          : { ...started, trade_no: "SUBUSR7NOnew" },
    });
    renderWithQueryClient(<AicliWalletPage />);
    const card = await screen.findByTestId("aicli-wallet-plan-3");
    expect(card).toHaveTextContent("Pro");
    // 没有生效的订阅：不显示「我的订阅」
    expect(screen.queryByTestId("aicli-wallet-mine")).toBeNull();
    // 这时邀请返利在左栏，排在订阅套餐前面
    expect(
      screen
        .getByTestId("aicli-wallet-invite")
        .compareDocumentPosition(screen.getByTestId("aicli-wallet-plans")) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();

    fireEvent.click(
      within(card).getByRole("button", {
        name: "aicli.wallet.plans.subscribe",
      }),
    );
    // 选付款方式之前不下单
    expect(calls("aicli_subscription_buy")).toEqual([]);
    const dialog = await screen.findByTestId("aicli-wallet-plan-dialog");
    fireEvent.click(
      within(dialog).getByRole("button", {
        name: "aicli.wallet.plans.balanceOption",
      }),
    );
    await waitFor(() =>
      expect(calls("aicli_subscription_buy")).toEqual([
        ["aicli_subscription_buy", { planId: 3, method: "balance" }],
      ]),
    );
    await waitFor(() =>
      expect(toastMock.success).toHaveBeenCalledWith(
        "aicli.wallet.plans.bought",
      ),
    );
    await waitFor(() =>
      expect(screen.queryByTestId("aicli-wallet-plan-dialog")).toBeNull(),
    );
    expect(screen.queryByTestId("aicli-wallet-pay-waiting")).toBeNull();

    fireEvent.click(
      within(screen.getByTestId("aicli-wallet-plan-3")).getByRole("button", {
        name: "aicli.wallet.plans.subscribe",
      }),
    );
    fireEvent.click(
      within(await screen.findByTestId("aicli-wallet-plan-dialog")).getByRole(
        "button",
        { name: "支付宝" },
      ),
    );
    expect(
      await screen.findByTestId("aicli-wallet-pay-waiting"),
    ).toBeInTheDocument();
    expect(calls("aicli_subscription_buy")[1]).toEqual([
      "aicli_subscription_buy",
      { planId: 3, method: "alipay" },
    ]);
  });

  it("shows the active subscription and marks its plan as current", async () => {
    respond({
      aicli_wallet_get: view({
        plans: [plan(), plan({ id: 4, title: "Max" })],
        subscriptions: [
          {
            id: 9,
            plan_id: 3,
            amount_total: 5_000_000,
            amount_used: 1_000_000,
            start_time: 1_762_560_000,
            end_time: 1_765_152_000,
            status: "active",
            next_reset_time: 0,
          },
          {
            id: 8,
            plan_id: 4,
            amount_total: 0,
            amount_used: 0,
            start_time: 1,
            end_time: 2,
            status: "expired",
            next_reset_time: 0,
          },
        ],
      }),
    });
    renderWithQueryClient(<AicliWalletPage />);
    const mine = await screen.findByTestId("aicli-wallet-mine");
    expect(mine).toHaveTextContent("Pro");
    expect(mine).toHaveTextContent("aicli.wallet.plans.active");
    expect(within(mine).getByRole("meter")).toHaveAttribute(
      "aria-valuenow",
      "20",
    );
    expect(
      within(screen.getByTestId("aicli-wallet-plan-3")).getByRole("button", {
        name: "aicli.wallet.plans.renew",
      }),
    ).toBeInTheDocument();
    expect(
      within(screen.getByTestId("aicli-wallet-plan-4")).getByRole("button", {
        name: "aicli.wallet.plans.subscribe",
      }),
    ).toBeInTheDocument();
    // 有订阅时兑换码留在左栏，邀请返利排到订阅套餐后面独占一行
    expect(screen.getByTestId("aicli-wallet-redeem")).toBeInTheDocument();
    const invite = screen.getByTestId("aicli-wallet-invite");
    expect(
      screen.getByTestId("aicli-wallet-plans").compareDocumentPosition(invite) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("redeems a code, transfers invitation rewards and lists the history", async () => {
    respond({ aicli_wallet_redeem: 500_000, aicli_aff_transfer: undefined });
    renderWithQueryClient(<AicliWalletPage />);

    const input = await screen.findByRole("textbox", {
      name: "aicli.wallet.redeem.title",
    });
    fireEvent.change(input, { target: { value: "  code-123  " } });
    fireEvent.click(
      screen.getByRole("button", { name: "aicli.wallet.redeem.submit" }),
    );
    await waitFor(() =>
      expect(calls("aicli_wallet_redeem")).toEqual([
        ["aicli_wallet_redeem", { key: "code-123" }],
      ]),
    );
    await waitFor(() => expect(input).toHaveValue(""));

    expect(
      screen.getByRole("textbox", { name: "aicli.wallet.invite.link" }),
    ).toHaveValue("https://aicliapi.com/sign-up?aff=abcd");
    fireEvent.click(
      await screen.findByRole("button", {
        name: "aicli.wallet.invite.transfer",
      }),
    );
    const transferButtons = await screen.findAllByRole("button", {
      name: "aicli.wallet.invite.transfer",
    });
    fireEvent.click(transferButtons[transferButtons.length - 1]);
    await waitFor(() =>
      expect(calls("aicli_aff_transfer")).toEqual([
        ["aicli_aff_transfer", { quota: 750_000 }],
      ]),
    );

    const table = await screen.findByRole("table");
    expect(table).toHaveTextContent("USR7NOabc");
    expect(table).toHaveTextContent("支付宝");
    expect(table).toHaveTextContent("aicli.wallet.history.status.success");
    // 很久以前下的单还是 pending：显示为已超时，不再写待支付
    expect(table.querySelector('[data-status="expired"]')).toHaveTextContent(
      "aicli.wallet.history.status.expired",
    );
    expect(table.querySelector('[data-status="pending"]')).toBeNull();
    expect(calls("aicli_wallet_history")[0]).toEqual([
      "aicli_wallet_history",
      { page: 1 },
    ]);
  });

  it("says so when the token is no longer valid", async () => {
    respond({
      aicli_wallet_get: () =>
        Promise.reject({ code: "token_invalid", message: "invalid" }),
    });
    renderWithQueryClient(<AicliWalletPage />);
    expect(
      await screen.findByText("aicli.account.tokenInvalid"),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("aicli-wallet-history")).toBeNull();
  });
});
