import type { AicliQuotaDisplay } from "@/types/aicli";

/**
 * 服务端额度 → 显示文本，换算口径与网页端一致（new-api v1.0.0-rc.34 的额度显示
 * 规则，按行为实现，未复制其代码）：
 *
 * - 美元额 = 额度 ÷ quota_per_unit；USD 汇率恒为 1，CNY 乘 usd_exchange_rate，
 *   CUSTOM 乘 custom_currency_exchange_rate 并用自定义符号；
 * - TOKENS 直接显示额度数（≥1000 缩成 k，保留 1 位小数）；
 * - |值| ≥ 1 保留 2 位小数，否则 4 位；去掉多余的 0；非零但小于可显示精度时
 *   显示为最小可显示值（不会显示成 0）。
 *
 * 没有显示设置（从未探测成功）时按网页端默认值：USD、每单位 500000。
 */
export const DEFAULT_QUOTA_DISPLAY: AicliQuotaDisplay = {
  quota_per_unit: 500_000,
  quota_display_type: "USD",
  usd_exchange_rate: 1,
  custom_currency_symbol: "¤",
  custom_currency_exchange_rate: 1,
  fetched_at: 0,
};

function trimZeros(text: string): string {
  if (!text.includes(".")) return text;
  return text.replace(/(\.\d*?)0+$/, "$1").replace(/\.$/, "");
}

function fractionDigits(value: number): number {
  return Math.abs(value) >= 1 ? 2 : 4;
}

function atLeastVisible(value: number, digits: number): number {
  const threshold = Math.pow(10, -digits);
  const abs = Math.abs(value);
  if (abs > 0 && abs < threshold) return value > 0 ? threshold : -threshold;
  return value;
}

/** 额度为空或不是数时返回 null（调用方显示「暂无数据」，不得显示 0）。 */
export function formatAicliQuota(
  quota: number | null | undefined,
  display: AicliQuotaDisplay | null | undefined,
  locale?: string,
): string | null {
  if (quota == null || !Number.isFinite(quota)) return null;
  const config = display ?? DEFAULT_QUOTA_DISPLAY;

  if (config.quota_display_type === "TOKENS") {
    if (Math.abs(quota) >= 1000) {
      return `${trimZeros((quota / 1000).toFixed(1))}k`;
    }
    return trimZeros(quota.toFixed(Math.abs(quota) >= 1 ? 0 : 4));
  }

  const perUnit =
    config.quota_per_unit > 0
      ? config.quota_per_unit
      : DEFAULT_QUOTA_DISPLAY.quota_per_unit;
  const usd = quota / perUnit;
  const rate =
    config.quota_display_type === "CNY"
      ? config.usd_exchange_rate
      : config.quota_display_type === "CUSTOM"
        ? config.custom_currency_exchange_rate
        : 1;
  const raw = usd * (rate > 0 ? rate : 1);
  const digits = fractionDigits(raw);
  const value = atLeastVisible(raw, digits);

  if (config.quota_display_type === "CUSTOM") {
    const number = new Intl.NumberFormat(locale, {
      minimumFractionDigits: 0,
      maximumFractionDigits: digits,
    }).format(value);
    const symbol =
      config.custom_currency_symbol.trim() ||
      DEFAULT_QUOTA_DISPLAY.custom_currency_symbol;
    return `${symbol} ${number}`;
  }

  return new Intl.NumberFormat(locale, {
    style: "currency",
    currency: config.quota_display_type === "CNY" ? "CNY" : "USD",
    currencyDisplay: "narrowSymbol",
    minimumFractionDigits: 0,
    maximumFractionDigits: digits,
  }).format(value);
}

/** /api/user/self 缓存里的额度字段（服务端原始响应 { data: {...} }）。 */
export interface AicliAccountQuota {
  remaining: number | null;
  used: number | null;
}

export function readAccountQuota(value: unknown): AicliAccountQuota {
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
  return { remaining: num("quota"), used: num("used_quota") };
}
