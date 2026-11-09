import { describe, expect, it } from "vitest";
import {
  DEFAULT_QUOTA_DISPLAY,
  formatAicliQuota,
  readAccountQuota,
} from "@/lib/aicliQuota";
import type { AicliQuotaDisplay } from "@/types/aicli";

const display = (patch: Partial<AicliQuotaDisplay>): AicliQuotaDisplay => ({
  ...DEFAULT_QUOTA_DISPLAY,
  ...patch,
});

describe("formatAicliQuota (web-compatible quota display)", () => {
  it("returns null instead of 0 when there is no value", () => {
    expect(formatAicliQuota(null, null)).toBeNull();
    expect(formatAicliQuota(undefined, null)).toBeNull();
    expect(formatAicliQuota(Number.NaN, null)).toBeNull();
  });

  it("defaults to USD with 500000 quota per unit", () => {
    expect(formatAicliQuota(1_000_000, null, "en-US")).toBe("$2");
    expect(formatAicliQuota(1_234_567, null, "en-US")).toBe("$2.47");
    // < 1：4 位小数
    expect(formatAicliQuota(12_345, null, "en-US")).toBe("$0.0247");
  });

  it("shows the smallest visible amount for tiny non-zero quotas", () => {
    expect(formatAicliQuota(1, null, "en-US")).toBe("$0.0001");
    expect(formatAicliQuota(0, null, "en-US")).toBe("$0");
  });

  it("converts CNY with the exchange rate", () => {
    const cny = display({ quota_display_type: "CNY", usd_exchange_rate: 7 });
    expect(formatAicliQuota(1_000_000, cny, "en-US")).toBe("¥14");
  });

  it("uses the custom symbol and rate", () => {
    const custom = display({
      quota_display_type: "CUSTOM",
      custom_currency_symbol: "点",
      custom_currency_exchange_rate: 100,
    });
    expect(formatAicliQuota(1_000_000, custom, "en-US")).toBe("点 200");
  });

  it("shows raw tokens with a k suffix", () => {
    const tokens = display({ quota_display_type: "TOKENS" });
    expect(formatAicliQuota(950, tokens)).toBe("950");
    expect(formatAicliQuota(1_500, tokens)).toBe("1.5k");
    expect(formatAicliQuota(2_000_000, tokens)).toBe("2000k");
  });

  it("respects a non-default quota per unit", () => {
    const perUnit = display({ quota_per_unit: 250_000 });
    expect(formatAicliQuota(1_000_000, perUnit, "en-US")).toBe("$4");
  });
});

describe("readAccountQuota", () => {
  it("reads remaining and used quota from the cached /api/user/self", () => {
    expect(
      readAccountQuota({ data: { quota: 10, used_quota: 3, username: "u" } }),
    ).toEqual({ remaining: 10, used: 3 });
  });

  it("returns null fields when the cache lacks them", () => {
    expect(readAccountQuota(null)).toEqual({ remaining: null, used: null });
    expect(readAccountQuota({ data: { quota: "10" } })).toEqual({
      remaining: null,
      used: null,
    });
  });
});
