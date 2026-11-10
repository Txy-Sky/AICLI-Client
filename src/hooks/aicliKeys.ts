/** AICLI react-query 键。所有键以 "aicli" 开头，便于整体失效。 */
export const aicliKeys = {
  all: ["aicli"] as const,
  account: ["aicli", "account"] as const,
  cache: (kind: string) => ["aicli", "cache", kind] as const,
  privacy: ["aicli", "privacy"] as const,
  /** F-09 隐私页的记录（与设置同前缀：改设置后一起刷新）。 */
  privacySummary: (days: number) =>
    ["aicli", "privacy", "summary", days] as const,
  privacyEvents: (days: number, category: string | null) =>
    ["aicli", "privacy", "events", days, category] as const,
  lines: ["aicli", "lines"] as const,
  catalog: ["aicli", "catalog"] as const,
  routes: ["aicli", "routes"] as const,
  profiles: ["aicli", "profiles"] as const,
  takeover: ["aicli", "takeover"] as const,
  usageSummary: ["aicli", "usage", "summary"] as const,
  requestBill: (requestId: string) =>
    ["aicli", "usage", "bill", requestId] as const,
};

/**
 * 后端 aicli-data-updated 事件的 kind → 需要失效的查询。
 * account：账户视图与账户缓存；groups/pricing：目录、路由、配置集（模型表
 * 由二者编译）；usage:YYYY-MM-DD：服务端用量汇总。
 */
export function keysForDataKind(
  kind: string,
): ReadonlyArray<readonly unknown[]> {
  if (kind === "account") {
    return [aicliKeys.account, aicliKeys.cache("account")];
  }
  if (kind === "groups" || kind === "pricing") {
    return [
      aicliKeys.cache(kind),
      aicliKeys.catalog,
      aicliKeys.routes,
      aicliKeys.profiles,
    ];
  }
  if (kind.startsWith("usage:")) {
    return [aicliKeys.usageSummary];
  }
  return [aicliKeys.all];
}
