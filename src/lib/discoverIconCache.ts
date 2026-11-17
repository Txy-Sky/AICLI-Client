import { invoke } from "@tauri-apps/api/core";

/**
 * 发现栏卡片图标的本地缓存（后端存在 ~/.aicli-client/cache/discover-icons/）。
 * 界面先查本地；没有时由 <img> 直接从图标地址加载，加载成功后再让后端存一份。
 */

/** 本次运行内查过的结果，免得每张卡片重复问后端 */
const lookups = new Map<string, Promise<string | null>>();
/** 已经请求过保存的地址（不论成败，本次运行只试一次） */
const stored = new Set<string>();

export function getCachedIcon(url: string): Promise<string | null> {
  let pending = lookups.get(url);
  if (!pending) {
    pending = invoke<string | null>("get_cached_discover_icon", { url })
      .then((value) => (typeof value === "string" ? value : null))
      .catch(() => null);
    lookups.set(url, pending);
  }
  return pending;
}

/** 图标在界面上加载成功：让后端下载一份存下（失败不影响显示） */
export function storeIcon(url: string): void {
  if (stored.has(url)) return;
  stored.add(url);
  void invoke<boolean>("cache_discover_icon", { url })
    .then((ok) => {
      // 存好了：下次（本次运行内）直接读本地
      if (ok) lookups.delete(url);
    })
    .catch(() => {});
}

/** 测试用 */
export function resetIconCacheMemo() {
  lookups.clear();
  stored.clear();
}
