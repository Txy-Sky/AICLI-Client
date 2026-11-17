/**
 * 用量页「上次的选择」（页签、时间范围、趋势 / 热力图指标）记在本机 localStorage：
 * 只是界面偏好，不进 settings.json、不随云同步。读不到、写不了（隐私模式）或值认不出时按默认值。
 */
export const USAGE_CHOICE_KEYS = {
  tab: "cc-switch.usage.tab",
  range: "cc-switch.usage.rangePreset",
  /** 指标条选中的指标（趋势图和热力图共用） */
  trendMetric: "cc-switch.usage.trendMetric",
  /** 请求日志每页几条 */
  logPageSize: "cc-switch.usage.logPageSize",
} as const;

/** 请求日志每页条数的可选值（存成字符串，和其他记下的选择一样） */
export const LOG_PAGE_SIZES = ["10", "20", "50"] as const;
export type LogPageSize = (typeof LOG_PAGE_SIZES)[number];

/** 指标条上的 4 个指标，也是趋势图 / 热力图能画的指标 */
export const USAGE_METRICS = ["cost", "requests", "tokens", "hitRate"] as const;
export type UsageMetric = (typeof USAGE_METRICS)[number];

export {
  readRememberedChoice,
  useRememberedChoice,
  writeRememberedChoice,
} from "@/lib/rememberedChoice";
