import type { AppId } from "@/lib/api";
import type {
  AicliAccountView,
  AicliProfilesView,
  AicliRouteProfile,
  AicliRoutesView,
} from "@/types/aicli";

/**
 * 某客户端实际使用的配置集：客户端专用绑定 > 账户默认 > 路由快照的默认 > 推荐预设。
 * 返回 id 与路由快照里的配置集（含模型表），快照还没取到时 profile 为空。
 */
export function resolveClientProfile(
  app: AppId,
  account: AicliAccountView | null | undefined,
  profiles: AicliProfilesView | null | undefined,
  routes: AicliRoutesView | null | undefined,
): { id: string; profile: AicliRouteProfile | undefined } {
  const appBinding = profiles?.bindings.find(
    (binding) => binding.scope === "app" && binding.scope_key === app,
  );
  const id =
    appBinding?.profile_id ??
    account?.active_profile_id ??
    routes?.active_profile_id ??
    "recommended";
  return { id, profile: routes?.profiles.find((entry) => entry.id === id) };
}
