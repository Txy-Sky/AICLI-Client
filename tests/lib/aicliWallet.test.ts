import type { TFunction } from "i18next";
import { describe, expect, it } from "vitest";
import {
  discountPercent,
  inviteLink,
  minAffTransfer,
  formatCountdown,
  parseTopupAmount,
  payMethodBrand,
  payMethodMark,
  planDuration,
  planPriceQuota,
  readAffStats,
  topupInputSymbol,
  topupOrderStatus,
  topupQuota,
  walletErrorText,
} from "@/lib/aicliWallet";
import type { AicliPlan, AicliQuotaDisplay } from "@/types/aicli";

const display = (
  patch: Partial<AicliQuotaDisplay> = {},
): AicliQuotaDisplay => ({
  quota_per_unit: 500_000,
  quota_display_type: "USD",
  usd_exchange_rate: 1,
  custom_currency_symbol: "¤",
  custom_currency_exchange_rate: 1,
  fetched_at: 0,
  ...patch,
});

const plan = (patch: Partial<AicliPlan> = {}): AicliPlan => ({
  id: 1,
  title: "Pro",
  subtitle: "",
  price_amount: 2,
  currency: "USD",
  duration_unit: "month",
  duration_value: 3,
  custom_seconds: 0,
  allow_balance_pay: null,
  max_purchase_per_user: 0,
  upgrade_group: "",
  total_amount: 0,
  quota_reset_period: "never",
  ...patch,
});

const t = ((key: string, options?: Record<string, unknown>) =>
  options && "count" in options
    ? `${key}:${String(options.count)}`
    : key) as unknown as TFunction;

describe("aicliWallet", () => {
  it("converts a top-up amount to quota the way the server does", () => {
    expect(topupQuota(10, display())).toBe(5_000_000);
    expect(topupQuota(10, display({ quota_per_unit: 1_000 }))).toBe(10_000);
    expect(topupQuota(10, display({ quota_display_type: "TOKENS" }))).toBe(10);
    // 没有显示设置时按网页端默认值
    expect(topupQuota(1, null)).toBe(500_000);
    expect(topupQuota(1, display({ quota_per_unit: 0 }))).toBe(500_000);
    expect(planPriceQuota(plan(), display())).toBe(1_000_000);
    expect(minAffTransfer(display({ quota_per_unit: 1_000 }))).toBe(1_000);
  });

  it("accepts only positive whole amounts", () => {
    expect(parseTopupAmount(" 25 ")).toBe(25);
    for (const bad of ["", "0", "-5", "1.5", "1e3", "abc", "1234567890"]) {
      expect(parseTopupAmount(bad)).toBeNull();
    }
  });

  it("reads discounts, invitation data and plan durations", () => {
    expect(discountPercent({ "100": 0.9 }, 100)).toBe(10);
    expect(discountPercent({ "100": 1 }, 100)).toBeNull();
    expect(discountPercent({ "100": 0 }, 100)).toBeNull();
    expect(discountPercent({}, 100)).toBeNull();
    expect(inviteLink("a b")).toBe("https://aicliapi.com/sign-up?aff=a%20b");
    expect(
      readAffStats({
        data: { aff_quota: 5, aff_history_quota: 9, aff_count: 2 },
      }),
    ).toEqual({ pending: 5, total: 9, invited: 2 });
    expect(readAffStats(null)).toEqual({
      pending: null,
      total: null,
      invited: null,
    });
    expect(planDuration(plan(), t)).toBe("aicli.wallet.duration.month:3");
    expect(
      planDuration(
        plan({ duration_unit: "custom", custom_seconds: 172_800 }),
        t,
      ),
    ).toBe("aicli.wallet.duration.day:2");
    expect(
      planDuration(plan({ duration_unit: "custom", custom_seconds: 7_200 }), t),
    ).toBe("aicli.wallet.duration.hour:2");
    expect(
      planDuration(plan({ duration_unit: "custom", custom_seconds: 0 }), t),
    ).toBe("aicli.wallet.duration.unknown");
  });

  it("labels the custom amount and payment methods", () => {
    expect(topupInputSymbol(display())).toBe("$");
    expect(topupInputSymbol(null)).toBe("$");
    expect(topupInputSymbol(display({ quota_display_type: "CNY" }))).toBeNull();
    expect(
      topupInputSymbol(display({ quota_display_type: "TOKENS" })),
    ).toBeNull();
    expect(payMethodBrand("alipay")).toBe("alipay");
    expect(payMethodBrand(" WxPay ")).toBe("wechat");
    expect(payMethodBrand("qqpay")).toBeNull();
    expect(payMethodMark("qqpay", " QQ 钱包")).toEqual({
      color: "#52525b",
      letter: "Q",
    });
    expect(payMethodMark("x", "").letter).toBe("?");
    expect(formatCountdown(5 * 60_000)).toBe("5:00");
    expect(formatCountdown(61_001)).toBe("1:02");
    expect(formatCountdown(999)).toBe("0:01");
    expect(formatCountdown(-5)).toBe("0:00");
  });

  it("shows an unpaid order as timed out once its validity has passed", () => {
    const created = 1_762_560_000;
    const at = (seconds: number) => (created + seconds) * 1000;
    const pending = { status: "pending", create_time: created };
    expect(topupOrderStatus(pending, at(60))).toBe("pending");
    // 5 分钟有效期 + 45 秒余量之内仍算待支付
    expect(topupOrderStatus(pending, at(345))).toBe("pending");
    expect(topupOrderStatus(pending, at(346))).toBe("expired");
    expect(
      topupOrderStatus({ status: "success", create_time: created }, at(9_999)),
    ).toBe("success");
    expect(
      topupOrderStatus({ status: "failed", create_time: created }, at(9_999)),
    ).toBe("failed");
    // 没有下单时间时不猜
    expect(
      topupOrderStatus({ status: "pending", create_time: 0 }, at(9_999)),
    ).toBe("pending");
  });

  it("maps failures to one sentence for the user", () => {
    expect(
      walletErrorText({ code: "wallet_declined", message: "兑换失败" }, t),
    ).toBe("兑换失败");
    expect(walletErrorText({ code: "rate_limited", message: "x" }, t)).toBe(
      "aicli.wallet.errors.rateLimited",
    );
    expect(walletErrorText({ code: "pay_window_open", message: "x" }, t)).toBe(
      "aicli.wallet.errors.payWindowOpen",
    );
    expect(walletErrorText({ code: "token_invalid", message: "x" }, t)).toBe(
      "aicli.account.tokenInvalid",
    );
    expect(walletErrorText(new Error("boom"), t)).toBe(
      "aicli.wallet.errors.generic",
    );
  });
});
