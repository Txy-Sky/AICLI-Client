import { invoke } from "@tauri-apps/api/core";
import type {
  AicliAccountView,
  AicliBindResult,
  AicliCacheKind,
  AicliCachedValue,
  AicliCatalog,
  AicliCommandError,
  AicliLinesView,
  AicliPrivacySettings,
  AicliPrivacyView,
  AicliProbe,
  AicliProfile,
  AicliProfileScope,
  AicliProfilesView,
  AicliRequestBill,
  AicliRoutesView,
  AicliTakeoverStatus,
  AicliUsageSummary,
} from "@/types/aicli";

/** 后端事件名（src-tauri/src/aicli/cache.rs 与 lib.rs）。 */
export const AICLI_EVENTS = {
  /** 缓存某一类数据更新完成；负载是 kind（account/groups/pricing/usage:YYYY-MM-DD）。 */
  dataUpdated: "aicli-data-updated",
  /** 绑定后自动接管的逐项结果（JSON 字符串，AicliBindResult 同形）。 */
  takeoverResults: "aicli-takeover-results",
  /** 深链 aicli://bind：打开绑定界面。 */
  openBind: "aicli-open-bind",
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

/** 对应 C3-11 的全部 AICLI 命令（参数名按 Tauri 约定用 camelCase）。 */
export const aicliApi = {
  // ---- 账户 ----
  getAccount: (): Promise<AicliAccountView> => invoke("aicli_get_account"),

  /** 绑定：accessToken 是网页端「个人设置」里生成的访问令牌（不是 sk- 模型密钥）。 */
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
  getUsageSummary: (): Promise<AicliUsageSummary> =>
    invoke("aicli_get_usage_summary"),

  getRequestBill: (requestId: string): Promise<AicliRequestBill | null> =>
    invoke("aicli_get_request_bill", { requestId }),

  // ---- 隐私 ----
  getPrivacy: (): Promise<AicliPrivacyView> => invoke("aicli_get_privacy"),

  setPrivacy: (settings: AicliPrivacySettings): Promise<void> =>
    invoke("aicli_set_privacy", {
      privacyEnabled: settings.privacyEnabled,
      privacyFailureMode: settings.privacyFailureMode,
      allowUnscannedMedia: settings.allowUnscannedMedia,
      privacyDetectCjkNames: settings.privacyDetectCjkNames ?? null,
    }),
};
