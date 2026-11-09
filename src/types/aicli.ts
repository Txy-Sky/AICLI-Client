/**
 * AICLI 命令的前端类型（对应 src-tauri/src/commands/aicli.rs 的 DTO）。
 *
 * 字段名与后端 serde 输出逐字一致：大多数视图是 snake_case；
 * `AicliBindResult` / `AicliTakeoverOutcome` 是 camelCase（后端
 * `#[serde(rename_all = "camelCase")]`）。
 */

/** 命令失败时 invoke 抛出的对象（AicliCommandError）。 */
export interface AicliCommandError {
  code: string;
  message: string;
}

export type AicliBindingState = "not_bound" | "bound" | "needs_rebind";

export interface AicliManagedToken {
  token_id: number;
  name: string;
  masked: string;
  created_by_client: boolean;
}

export interface AicliAccountView {
  state: AicliBindingState;
  user_id: number | null;
  username: string | null;
  user_group: string | null;
  selected_line_id: string | null;
  active_profile_id: string | null;
  managed_tokens: AicliManagedToken[];
  /** 额度显示设置（最近一次成功探测的 /api/status）；从未探测成功时为 null。 */
  quota_display: AicliQuotaDisplay | null;
}

export type AicliQuotaDisplayType = "USD" | "CNY" | "TOKENS" | "CUSTOM";

export interface AicliQuotaDisplay {
  quota_per_unit: number;
  quota_display_type: AicliQuotaDisplayType;
  usd_exchange_rate: number;
  custom_currency_symbol: string;
  custom_currency_exchange_rate: number;
  fetched_at: number;
}

export interface AicliTakeoverOutcome {
  app: string;
  success: boolean;
  errorCode: string | null;
  errorMessage: string | null;
}

export interface AicliBindResult {
  account: AicliAccountView | null;
  accountError: string | null;
  proxyStarted: boolean;
  proxyError: string | null;
  runtimeRefreshError: string | null;
  takeoverTargetsError: string | null;
  takeovers: AicliTakeoverOutcome[];
}

/** `aicli_refresh` 的缓存条目；`value` 是服务端原始响应。 */
export type AicliCacheKind = "account" | "groups" | "pricing";

export interface AicliCachedValue<T = unknown> {
  kind: string;
  value: T;
  fetched_at: number;
  stale: boolean;
  last_error: string | null;
}

export type AicliCompatibility =
  | "full"
  | "partial"
  | "text_only"
  | "unavailable"
  | "unverified";

export interface AicliCatalogGroup {
  description: string;
  ratio: number | null;
  ratio_known: boolean;
}

export interface AicliCatalogModel {
  groups: string[];
  /** 客户端 → 线协议（openai-response / openai / anthropic / gemini）。 */
  wire: Record<string, string>;
  compat: Record<string, AicliCompatibility>;
  /** 服务端定价原始行（只作展示，不是本地算出的实扣）。 */
  pricing: unknown;
}

export interface AicliCatalog {
  groups: Record<string, AicliCatalogGroup>;
  models: Record<string, AicliCatalogModel>;
  auto_available: boolean;
  auto_groups: string[];
}

export type AicliSelectionMode = "fixed_priority" | "cost_first";

export interface AicliModelBinding {
  model: string;
  source_group: string | null;
  alias: string | null;
}

export interface AicliProfileBody {
  selected_groups: string[];
  selection_mode: AicliSelectionMode;
  model_bindings: AicliModelBinding[];
  resolved_conflicts: string[];
  line_policy: string;
  allow_fallback: boolean;
}

export interface AicliProfile {
  id: string;
  name: string;
  /** system（预设，只读）/ system_copy（从预设新建）/ user。 */
  source: string;
  preset_id: string | null;
  revision: number;
  body: AicliProfileBody;
}

export interface AicliScopeBinding {
  scope: string;
  scope_key: string;
  profile_id: string;
}

export interface AicliProfilesView {
  profiles: AicliProfile[];
  presets: AicliProfile[];
  bindings: AicliScopeBinding[];
  active_profile_id: string | null;
}

export interface AicliModelRoute {
  public_model: string;
  upstream_model: string;
  ordered_groups: string[];
}

export interface AicliRepairStatus {
  conservative: boolean;
  needs_repair: boolean;
  missing_groups: string[];
  affected_models: string[];
}

export interface AicliRouteProfile {
  id: string;
  name: string;
  source: string;
  preset_id: string | null;
  revision: number;
  routes: AicliModelRoute[];
  repair: AicliRepairStatus;
}

export interface AicliRoutesView {
  fingerprint: string;
  active_profile_id: string | null;
  selected_line_id: string;
  profiles: AicliRouteProfile[];
}

export interface AicliLineHealth {
  checked_at: number;
  status: string;
  latency_ms: number | null;
  http_status: number | null;
  error_code: string | null;
  next_check_at: number | null;
}

export interface AicliLine {
  id: string;
  label_key: string;
  transport: string;
  priority: number;
  first_byte_s: number;
  idle_s: number;
  stream_total_s: number;
  non_stream_s: number;
  health: AicliLineHealth | null;
}

export interface AicliLinesView {
  selected_line_id: string;
  lines: AicliLine[];
}

export interface AicliProbe {
  line_id: string;
  status: string;
  checked_at: number;
  latency_ms: number | null;
  http_status: number | null;
  error_code: string | null;
  next_check_at: number | null;
}

export interface AicliTakeoverStatus {
  app: string;
  state: string;
  taken_over_at: number | null;
}

export interface AicliUsageDay {
  day: string;
  quota: number;
  requests: number;
  is_final: boolean;
  fetched_at: number;
  stale: boolean;
}

export interface AicliUsageSummary {
  days: AicliUsageDay[];
  total_quota: number;
  total_requests: number;
}

export interface AicliRequestBill {
  request_id: string;
  created_at: number;
  app_type: string;
  model: string;
  status_code: number;
  line_id: string | null;
  profile_id: string | null;
  upstream_request_id: string | null;
  server_group: string | null;
  /** 服务端实扣额度；`billed_at` 为空时尚未记账。 */
  billed_quota: number | null;
  billed_at: number | null;
}

export type AicliPrivacyFailureMode = "block" | "pass";

export interface AicliPrivacyView {
  bound: boolean;
  privacy_enabled: boolean;
  privacy_failure_mode: AicliPrivacyFailureMode;
  allow_unscanned_media: boolean;
  privacy_detect_cjk_names: boolean;
  privacy_detect_cjk_names_available: boolean;
  /** 隐私引擎本次运行以来的计数；代理未运行时为 null。 */
  engine: AicliPrivacyEngine | null;
}

export interface AicliPrivacyEngine {
  masked_values: number;
  /** 可用性优先模式下引擎失败、原文放行的次数。 */
  degraded: number;
  /** 隐私优先模式下引擎失败、请求被阻断的次数。 */
  engine_blocked: number;
  media_passed: number;
}

export interface AicliPrivacySettings {
  privacyEnabled: boolean;
  privacyFailureMode: AicliPrivacyFailureMode;
  allowUnscannedMedia: boolean;
  /** 不传则保持原值（后端 Option<bool>）。 */
  privacyDetectCjkNames?: boolean;
}

/**
 * 配置集作用域（后端 aicli_set_active_profile 只接受 global 与 app；
 * 项目、会话级只走代码接口，首版不做 UI）。global 的 scopeKey 必须为空，
 * app 的 scopeKey 是客户端 id（claude / codex / gemini …）。
 */
export type AicliProfileScope = "global" | "app";
