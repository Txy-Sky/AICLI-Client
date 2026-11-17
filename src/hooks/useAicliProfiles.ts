import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { aicliApi } from "@/lib/api/aicli";
import type { AicliProfile, AicliProfileScope } from "@/types/aicli";
import { aicliKeys } from "./aicliKeys";

/** 配置集列表（用户配置集 + 系统预设 + 作用域绑定）。只存本机，不跨设备同步。 */
export function useAicliProfiles(enabled = true) {
  return useQuery({
    queryKey: aicliKeys.profiles,
    queryFn: () => aicliApi.getProfiles(),
    enabled,
    placeholderData: (previous) => previous,
  });
}

/** 每个配置集编译后的模型路由与修复状态。 */
export function useAicliRoutes(enabled = true) {
  return useQuery({
    queryKey: aicliKeys.routes,
    queryFn: () => aicliApi.getRoutes(),
    enabled,
    placeholderData: (previous) => previous,
  });
}

/** 服务端模型目录（由分组与定价缓存编译；无缓存时为 null）。 */
export function useAicliCatalog(enabled = true) {
  return useQuery({
    queryKey: aicliKeys.catalog,
    queryFn: () => aicliApi.getCatalog(),
    enabled,
    placeholderData: (previous) => previous,
  });
}

function useInvalidateProfiles() {
  const queryClient = useQueryClient();
  return () =>
    Promise.all([
      queryClient.invalidateQueries({ queryKey: aicliKeys.profiles }),
      queryClient.invalidateQueries({ queryKey: aicliKeys.routes }),
      queryClient.invalidateQueries({ queryKey: aicliKeys.account }),
    ]);
}

/** 保存（新建或更新）。revision 冲突时后端返回 revision_conflict。 */
export function useAicliSaveProfile() {
  const invalidate = useInvalidateProfiles();
  return useMutation({
    mutationFn: (profile: AicliProfile) => aicliApi.saveProfile(profile),
    onSettled: invalidate,
  });
}

export function useAicliDeleteProfile() {
  const invalidate = useInvalidateProfiles();
  return useMutation({
    mutationFn: ({ id, revision }: { id: string; revision: number }) =>
      aicliApi.deleteProfile(id, revision),
    onSettled: invalidate,
  });
}

/** 设为默认（global）或某客户端专用（app）；id 为 null 清除该作用域。 */
export function useAicliSetActiveProfile() {
  const invalidate = useInvalidateProfiles();
  return useMutation({
    mutationFn: ({
      scope,
      scopeKey,
      id,
    }: {
      scope: AicliProfileScope;
      scopeKey: string;
      id: string | null;
    }) => aicliApi.setActiveProfile(scope, scopeKey, id),
    onSettled: invalidate,
  });
}

/** 各客户端的接管状态。 */
export function useAicliTakeoverStatus(enabled = true) {
  return useQuery({
    queryKey: aicliKeys.takeover,
    queryFn: () => aicliApi.takeoverStatus(),
    enabled,
    placeholderData: (previous) => previous,
  });
}

export function useAicliTakeOver() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ app, confirmed }: { app: string; confirmed: boolean }) =>
      aicliApi.takeOver(app, confirmed),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: aicliKeys.takeover }),
  });
}

export function useAicliRelease() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (app: string) => aicliApi.release(app),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: aicliKeys.takeover }),
  });
}
