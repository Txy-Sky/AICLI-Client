import { invoke } from "@tauri-apps/api/core";
import type {
  AicliAccountView,
  AicliBindResult,
  AicliCacheKind,
  AicliCachedValue,
  AicliCatalog,
  AicliCommandError,
  AicliLinesView,
  AicliPayStarted,
  AicliPrivacyRule,
  AicliPrivacySettings,
  AicliPrivacyView,
  AicliPrivacyCategory,
  AicliPrivacyEventPage,
  AicliPrivacyPreview,
  AicliPrivacySummary,
  AicliProbe,
  AicliProfile,
  AicliProfileScope,
  AicliProfilesView,
  AicliRequestBill,
  AicliRoutesView,
  AicliTakeoverStatus,
  AicliTopupHistory,
  AicliUsageSummary,
  AicliWalletView,
} from "@/types/aicli";

/** 后端事件名（src-tauri/src/aicli/cache.rs 与 lib.rs）。 */
export const AICLI_EVENTS = {
  /** 缓存某一类数据更新完成；负载是 kind（account/groups/pricing/usage:YYYY-MM-DD）。 */
  dataUpdated: "aicli-data-updated",
  /** 绑定后自动接管的逐项结果（JSON 字符串，AicliBindResult 同形）。 */
  takeoverResults: "aicli-takeover-results",
  /** 深链 aicli://bind：打开绑定界面。 */
  openBind: "aicli-open-bind",
  /** 支付窗口结束（回到站点或被关闭）；负载是 AicliPayFinished。 */
  payFinished: "aicli-pay-finished",
} as const;

/** invoke 抛出的错误是否为 AICLI 命令错误（带机器码）。 */
export function isAicliCommandError(
  error: unknown,
): error is AicliCommandError {
  return (
    typeof error === "object" &&
    error !== null &&
    typeof (error as { code?: unknown }).code === "string" &&
    typeof (error as { message?: unknown }).message === "string"
  );
}

/** 取 AICLI 错误机器码；不是 AICLI 错误时返回 undefined。 */
export function aicliErrorCode(error: unknown): string | undefined {
  return isAicliCommandError(error) ? error.code : undefined;
}

/** 对应 C3-11 的全部 AICLI 命令及 F-06 新增的自定义遮盖项（参数名按 Tauri 约定用 camelCase）。 */
export const aicliApi = {
  // ---- 账户 ----
  getAccount: (): Promise<AicliAccountView> => invoke("aicli_get_account"),

  /** 绑定：accessToken 是网页端「安全与访问」→「访问令牌」里生成的访问令牌（不是 sk- 模型密钥）。 */
  bind: (accessToken: string, line: string): Promise<AicliBindResult> =>
    invoke("aicli_bind", { accessToken, line }),

  unbind: (deleteCreatedTokens: boolean): Promise<void> =>
    invoke("aicli_unbind", { deleteCreatedTokens }),

  /** 返回当前缓存（立即）并在后台刷新；kind=all 刷新三类。 */
  refresh: (kind: AicliCacheKind | "all"): Promise<AicliCachedValue[]> =>
    invoke("aicli_refresh", { kind }),

  // ---- 模型目录与路由 ----
  getCatalog: (): Promise<AicliCatalog | null> => invoke("aicli_get_catalog"),

  getRoutes: (): Promise<AicliRoutesView> => invoke("aicli_get_routes"),

  // ---- 线路 ----
  listLines: (): Promise<AicliLinesView> => invoke("aicli_list_lines"),

  selectLine: (id: string): Promise<void> =>
    invoke("aicli_select_line", { id }),

  probeLines: (): Promise<AicliProbe[]> => invoke("aicli_probe_lines"),

  /** 接管 Claude Code 时 /model 是否保留它自带的行；已接管时立即重写。 */
  setClaudeKeepBuiltinModels: (keep: boolean): Promise<void> =>
    invoke("aicli_set_claude_keep_builtin_models", { keep }),

  // ---- 配置集 ----
  getProfiles: (): Promise<AicliProfilesView> => invoke("aicli_get_profiles"),

  saveProfile: (profile: AicliProfile): Promise<AicliProfile> =>
    invoke("aicli_save_profile", { profile }),

  deleteProfile: (id: string, revision: number): Promise<void> =>
    invoke("aicli_delete_profile", { id, revision }),

  /** id 为 null 时清除该作用域的绑定；global 的 scopeKey 必须为空字符串。 */
  setActiveProfile: (
    scope: AicliProfileScope,
    scopeKey: string,
    id: string | null,
  ): Promise<void> =>
    invoke("aicli_set_active_profile", { scope, scopeKey, id }),

  // ---- 客户端接管 ----
  takeoverStatus: (): Promise<AicliTakeoverStatus[]> =>
    invoke("aicli_takeover_status"),

  /**
   * confirmed=false 时照常接管，但客户端配置里有旧 CC Switch 代理设置时返回
   * cc_switch_conflict；用户确认覆盖后再以 confirmed=true 重试。
   */
  takeOver: (app: string, confirmed: boolean): Promise<void> =>
    invoke("aicli_take_over", { app, confirmed }),

  release: (app: string): Promise<void> => invoke("aicli_release", { app }),

  // ---- 用量与账单 ----
  /** days：同时补拉最近几天的服务端用量（F-10 首页趋势，默认只刷新今天）。 */
  getUsageSummary: (days?: number): Promise<AicliUsageSummary> =>
    days === undefined
      ? invoke("aicli_get_usage_summary")
      : invoke("aicli_get_usage_summary", { days }),

  getRequestBill: (requestId: string): Promise<AicliRequestBill | null> =>
    invoke("aicli_get_request_bill", { requestId }),

  // ---- 隐私 ----
  getPrivacy: (): Promise<AicliPrivacyView> => invoke("aicli_get_privacy"),

  // ---- F-09 隐私脱敏页 ----
  privacySummary: (days: number): Promise<AicliPrivacySummary> =>
    invoke("aicli_privacy_summary", { days }),

  privacyEvents: (params: {
    days: number;
    category?: AicliPrivacyCategory | null;
    beforeId?: number | null;
    limit?: number;
  }): Promise<AicliPrivacyEventPage> =>
    invoke("aicli_privacy_events", {
      days: params.days,
      category: params.category ?? null,
      beforeId: params.beforeId ?? null,
      limit: params.limit ?? null,
    }),

  privacyPreview: (text: string): Promise<AicliPrivacyPreview> =>
    invoke("aicli_privacy_preview", { text }),

  privacyClearEvents: (): Promise<void> => invoke("aicli_privacy_clear_events"),

  /** 整体替换自定义遮盖词与规则（后端整理并校验，失败返回 invalid_input）。 */
  setPrivacyCustom: (
    words: string[],
    rules: AicliPrivacyRule[],
  ): Promise<void> => invoke("aicli_set_privacy_custom", { words, rules }),

  setPrivacy: (settings: AicliPrivacySettings): Promise<void> =>
    invoke("aicli_set_privacy", {
      privacyEnabled: settings.privacyEnabled,
      privacyFailureMode: settings.privacyFailureMode,
      allowUnscannedMedia: settings.allowUnscannedMedia,
      privacyDetectCjkNames: settings.privacyDetectCjkNames ?? null,
    }),

  // ---- 钱包（W 阶段）----
  walletGet: (): Promise<AicliWalletView> => invoke("aicli_wallet_get"),

  /** 充值 amount 的实付金额（服务端格式化的两位小数）。 */
  walletQuote: (amount: number): Promise<string> =>
    invoke("aicli_wallet_quote", { amount }),

  /** 下单并打开支付窗口。 */
  walletPay: (amount: number, method: string): Promise<AicliPayStarted> =>
    invoke("aicli_wallet_pay", { amount, method }),

  /** 关闭支付窗口（订单超时或用户不再等待）；没有窗口时返回 false。 */
  walletClosePay: (): Promise<boolean> => invoke("aicli_wallet_close_pay"),

  /** 兑换码充值；返回到账的原始额度。 */
  walletRedeem: (key: string): Promise<number> =>
    invoke("aicli_wallet_redeem", { key }),

  walletHistory: (page: number): Promise<AicliTopupHistory> =>
    invoke("aicli_wallet_history", { page }),

  /** 一笔充值订单的状态；未查到时为 null。 */
  walletOrderStatus: (tradeNo: string): Promise<string | null> =>
    invoke("aicli_wallet_order_status", { tradeNo }),

  /** method 为 "balance" 时用余额购买，否则走易支付并打开支付窗口。 */
  subscriptionBuy: (planId: number, method: string): Promise<AicliPayStarted> =>
    invoke("aicli_subscription_buy", { planId, method }),

  /** 把邀请奖励划转到余额；quota 为原始额度。 */
  affTransfer: (quota: number): Promise<void> =>
    invoke("aicli_aff_transfer", { quota }),
};
