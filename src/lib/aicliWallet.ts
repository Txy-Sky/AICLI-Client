import type { TFunction } from "i18next";
import { WEBSITE_URL } from "@/config/branding";
import { aicliErrorCode, isAicliCommandError } from "@/lib/api/aicli";
import { DEFAULT_QUOTA_DISPLAY } from "@/lib/aicliQuota";
import type { AicliPlan, AicliQuotaDisplay } from "@/types/aicli";

/** 余额支付的支付方式标识（后端 commands/aicli/wallet.rs 的 BALANCE_METHOD）。 */
export const BALANCE_METHOD = "balance";

/** 一个金额单位对应的原始额度（无显示设置时按网页端默认值）。 */
export function quotaPerUnit(
  display: AicliQuotaDisplay | null | undefined,
): number {
  const value = display?.quota_per_unit ?? 0;
  return value > 0 ? value : DEFAULT_QUOTA_DISPLAY.quota_per_unit;
}

/**
 * 充值数量 → 到账的原始额度，口径与 rc.34 一致：TOKENS 显示时数量就是额度，
 * 其余显示方式下数量是金额单位，乘 quota_per_unit。
 */
export function topupQuota(
  amount: number,
  display: AicliQuotaDisplay | null | undefined,
): number {
  if (display?.quota_display_type === "TOKENS") return amount;
  return amount * quotaPerUnit(display);
}

/** 套餐标价 → 原始额度（标价以美元计，和网页端用同一套显示换算）。 */
export function planPriceQuota(
  plan: AicliPlan,
  display: AicliQuotaDisplay | null | undefined,
): number {
  return plan.price_amount * quotaPerUnit(display);
}

/**
 * 自定义充值输入框前的单位符号：只有美元显示时数量与显示单位一致，写「$」；
 * 其余显示方式下输入的数量和显示的币种不是一回事，不加符号（到账额度另行显示）。
 */
export function topupInputSymbol(
  display: AicliQuotaDisplay | null | undefined,
): string | null {
  const type = display?.quota_display_type ?? "USD";
  return type === "USD" ? "$" : null;
}

const METHOD_MARK_COLORS: Record<string, string> = {
  alipay: "#1677ff",
  wxpay: "#07853f",
};

/** 支付方式的色块标记：已知方式用品牌色，其余用中性灰；白字都满足对比度。 */
export function payMethodMark(type: string, name: string) {
  return {
    color: METHOD_MARK_COLORS[type] ?? "#52525b",
    letter: Array.from(name.trim())[0] ?? "?",
  };
}

/** 邀请奖励至少攒够一个额度单位才能划转（服务端规则）。 */
export function minAffTransfer(
  display: AicliQuotaDisplay | null | undefined,
): number {
  return quotaPerUnit(display);
}

/** 输入框里的充值数量：只接受正整数。 */
export function parseTopupAmount(text: string): number | null {
  const trimmed = text.trim();
  if (!/^\d{1,9}$/.test(trimmed)) return null;
  const value = Number(trimmed);
  return value > 0 ? value : null;
}

/** 预设数量的折扣百分比（九折 → 10）；没有折扣时为 null。 */
export function discountPercent(
  discounts: Record<string, number>,
  amount: number,
): number | null {
  const ratio = discounts[String(amount)];
  if (typeof ratio !== "number" || !(ratio > 0) || ratio >= 1) return null;
  return Math.round((1 - ratio) * 100);
}

export function inviteLink(code: string): string {
  return `${WEBSITE_URL}/sign-up?aff=${encodeURIComponent(code)}`;
}

export interface AicliAffStats {
  /** 待划转的奖励（原始额度）。 */
  pending: number | null;
  /** 累计获得的奖励（原始额度）。 */
  total: number | null;
  invited: number | null;
}

/** 从 /api/user/self 的缓存里读邀请统计（服务端原始响应 { data: {...} }）。 */
export function readAffStats(value: unknown): AicliAffStats {
  const data =
    value && typeof value === "object"
      ? (value as { data?: unknown }).data
      : undefined;
  const record =
    data && typeof data === "object" ? (data as Record<string, unknown>) : {};
  const num = (key: string) =>
    typeof record[key] === "number" && Number.isFinite(record[key])
      ? (record[key] as number)
      : null;
  return {
    pending: num("aff_quota"),
    total: num("aff_history_quota"),
    invited: num("aff_count"),
  };
}

const DURATION_UNITS = ["year", "month", "day", "hour"] as const;

/** 套餐时长的显示文本。 */
export function planDuration(plan: AicliPlan, t: TFunction): string {
  const unit = DURATION_UNITS.find((entry) => entry === plan.duration_unit);
  if (unit && plan.duration_value > 0) {
    return t(`aicli.wallet.duration.${unit}`, { count: plan.duration_value });
  }
  if (plan.custom_seconds >= 86_400) {
    return t("aicli.wallet.duration.day", {
      count: Math.round(plan.custom_seconds / 86_400),
    });
  }
  if (plan.custom_seconds > 0) {
    return t("aicli.wallet.duration.hour", {
      count: Math.max(1, Math.round(plan.custom_seconds / 3_600)),
    });
  }
  return t("aicli.wallet.duration.unknown");
}

const ERROR_KEYS: Record<string, string> = {
  rate_limited: "aicli.wallet.errors.rateLimited",
  network: "aicli.wallet.errors.network",
  token_invalid: "aicli.account.tokenInvalid",
  not_bound: "aicli.account.tokenInvalid",
  pay_window_open: "aicli.wallet.errors.payWindowOpen",
  pay_window: "aicli.wallet.errors.payWindow",
  wallet_disabled: "aicli.wallet.errors.disabled",
  wallet_amount: "aicli.wallet.errors.amount",
  wallet_below_minimum: "aicli.wallet.errors.belowMinimum",
  wallet_method: "aicli.wallet.errors.method",
  wallet_redeem_key: "aicli.wallet.errors.redeemKey",
  wallet_checkout: "aicli.wallet.errors.checkout",
};

/**
 * 钱包操作失败时给用户看的一句话。服务端拒绝（wallet_declined）时直接用
 * 服务端给的原因（例如「充值数量不能小于 10」）；其余按机器码取文案。
 */
export function walletErrorText(error: unknown, t: TFunction): string {
  const code = aicliErrorCode(error);
  if (code === "wallet_declined" && isAicliCommandError(error)) {
    return error.message;
  }
  const key = code ? ERROR_KEYS[code] : undefined;
  return t(key ?? "aicli.wallet.errors.generic");
}
