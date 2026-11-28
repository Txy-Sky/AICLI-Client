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
  /** 仅 missing_scopes：服务端拒绝的访问令牌权限（"resource:action"）；为空表示客户端不认识的权限。 */
  scopes?: string[];
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
  "full" | "partial" | "text_only" | "unavailable" | "unverified";

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
  /** 用户排除的模型（不进路由，也不算冲突）。 */
  excluded_models: string[];
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
  /** 服务端分组的倍率（本机缓存的分组列表）；未记账或缓存里没有时为空。 */
  group_ratio: number | null;
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
  /** 用户自定义的遮盖词与规则（只存本机）。 */
  custom_words: string[];
  custom_rules: AicliPrivacyRule[];
  /** 凭据实际存储后端；file_fallback = 系统钥匙串不可用，已降级为本地文件。 */
  secret_backend: "keychain" | "file_fallback" | null;
}

/** 自定义遮盖规则（后端 UserRule，camelCase）：前缀规则或正则规则二选一。 */
export interface AicliPrivacyRule {
  kind: string;
  prefix?: string;
  regex?: string;
}

export interface AicliPrivacyEngine {
  masked_values: number;
  /** 可用性优先模式下引擎失败、原文放行的次数。 */
  degraded: number;
  /** 隐私优先模式下引擎失败、请求被阻断的次数。 */
  engine_blocked: number;
  media_passed: number;
}

/** F-09 隐私页：四组类别。 */
export type AicliPrivacyCategory = "secret" | "personal" | "network" | "custom";

export interface AicliPrivacyCount {
  key: string;
  count: number;
}

export interface AicliPrivacyDay {
  day: string;
  values: number;
  requests: number;
}

/** 最近 N 天（本地日期）的脱敏汇总；只有数量与类别，不含原值。 */
export interface AicliPrivacySummary {
  days: AicliPrivacyDay[];
  total_values: number;
  distinct_values: number;
  requests: number;
  blocked: number;
  degraded: number;
  media_passed: number;
  categories: AicliPrivacyCount[];
  kinds: AicliPrivacyCount[];
  apps: AicliPrivacyCount[];
  range_days: number;
  retention_days: number;
}

export type AicliPrivacyOutcome =
  "masked" | "degraded" | "blocked" | "media_passed";

export interface AicliPrivacyEvent {
  id: number;
  /** 毫秒时间戳。 */
  ts: number;
  app_type: string | null;
  line_id: string | null;
  outcome: AicliPrivacyOutcome;
  reason: string | null;
  value_count: number;
  distinct_count: number;
  kinds: AicliPrivacyCount[];
}

export interface AicliPrivacyEventPage {
  items: AicliPrivacyEvent[];
  next_before_id: number | null;
}

export interface AicliPrivacyPreview {
  masked: string;
  count: number;
  kinds: AicliPrivacyCount[];
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

// ---- 钱包（W 阶段）：对应 src-tauri/src/aicli/wallet.rs 与 commands/aicli/wallet.rs ----

export interface AicliPayMethod {
  name: string;
  /** 易支付的支付方式标识（alipay / wxpay …）。 */
  type: string;
  color: string | null;
  min_topup: number | null;
}

/** 线上充值设置（/api/user/topup/info 中客户端用到的部分）。 */
export interface AicliTopupInfo {
  /** 易支付已开启且至少配置了一种易支付方式。 */
  online_topup: boolean;
  redemption: boolean;
  methods: AicliPayMethod[];
  min_topup: number;
  amount_options: number[];
  /** 预设数量 → 价格系数（0.9 即九折）；键是数量的字符串。 */
  discounts: Record<string, number>;
  /** 管理员配置的外部购买链接（https）。 */
  topup_link: string | null;
}

export interface AicliPlan {
  id: number;
  title: string;
  subtitle: string;
  price_amount: number;
  currency: string;
  /** year / month / day / hour / custom。 */
  duration_unit: string;
  duration_value: number;
  custom_seconds: number;
  /** null 表示服务端未设置（按允许处理，最终以服务端为准）。 */
  allow_balance_pay: boolean | null;
  max_purchase_per_user: number;
  upgrade_group: string;
  /** 原始额度；0 表示不限。 */
  total_amount: number;
  quota_reset_period: string;
}

export interface AicliSubscription {
  id: number;
  plan_id: number;
  amount_total: number;
  amount_used: number;
  /** 秒级时间戳。 */
  start_time: number;
  end_time: number;
  status: string;
  next_reset_time: number;
}

/** 钱包页除余额外的全部数据；某一部分读取失败时列在 failed 里并留空。 */
export interface AicliWalletView {
  topup: AicliTopupInfo | null;
  plans: AicliPlan[];
  subscriptions: AicliSubscription[];
  billing_preference: string | null;
  aff_code: string | null;
  failed: string[];
}

export interface AicliPayStarted {
  trade_no: string | null;
  /** 收银台主机名；余额支付时为空。 */
  host: string | null;
  /** 为 true 时无需等待（余额支付已完成）。 */
  completed: boolean;
}

export interface AicliTopupOrder {
  id: number;
  amount: number;
  money: number;
  trade_no: string;
  payment_method: string;
  /** 秒级时间戳。 */
  create_time: number;
  complete_time: number;
  status: string;
}

export interface AicliTopupHistory {
  items: AicliTopupOrder[];
  total: number;
}

export type AicliPayKind = "topup" | "subscription";

/** 支付窗口结束事件（aicli-pay-finished）的负载。 */
export interface AicliPayFinished {
  kind: AicliPayKind;
  trade_no: string | null;
  outcome: "success" | "fail" | "unknown" | "closed";
}
