import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";
import { aicliApi, AICLI_EVENTS } from "@/lib/api/aicli";
import type {
  AicliCacheKind,
  AicliCachedValue,
  AicliPrivacyRule,
  AicliPrivacySettings,
} from "@/types/aicli";
import { useTauriEvent } from "./useTauriEvent";
import { aicliKeys, keysForDataKind } from "./aicliKeys";

/** 绑定状态与账户概要（本地数据库，不走网络）。 */
export function useAicliAccount() {
  return useQuery({
    queryKey: aicliKeys.account,
    queryFn: () => aicliApi.getAccount(),
    placeholderData: (previous) => previous,
  });
}

/**
 * 服务端缓存的一类数据（account/groups/pricing）。缓存优先：命令立即返回
 * 已存的快照（可能为空）并在后台刷新，刷新完成后 aicli-data-updated 事件
 * 让本查询原地更新（见 useAicliEventBridge）。未绑定时不查询。
 */
export function useAicliCachedValue<T = unknown>(
  kind: AicliCacheKind,
  enabled = true,
) {
  return useQuery({
    queryKey: aicliKeys.cache(kind),
    queryFn: async (): Promise<AicliCachedValue<T> | null> => {
      const values = await aicliApi.refresh(kind);
      return (values[0] as AicliCachedValue<T> | undefined) ?? null;
    },
    enabled,
    placeholderData: (previous) => previous,
  });
}

function invalidateAll(queryClient: QueryClient) {
  return queryClient.invalidateQueries({ queryKey: aicliKeys.all });
}

/** 绑定：成功后所有 AICLI 查询失效（账户、线路、配置集都随绑定变化）。 */
export function useAicliBind() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      accessToken,
      line,
    }: {
      accessToken: string;
      line: string;
    }) => aicliApi.bind(accessToken, line),
    onSettled: () => invalidateAll(queryClient),
  });
}

export function useAicliUnbind() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (deleteCreatedTokens: boolean) =>
      aicliApi.unbind(deleteCreatedTokens),
    onSettled: () => invalidateAll(queryClient),
  });
}

export function useAicliPrivacy() {
  return useQuery({
    queryKey: aicliKeys.privacy,
    queryFn: () => aicliApi.getPrivacy(),
    placeholderData: (previous) => previous,
  });
}

export function useAicliSetPrivacy() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (settings: AicliPrivacySettings) =>
      aicliApi.setPrivacy(settings),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: aicliKeys.privacy }),
  });
}

export function useAicliSetPrivacyCustom() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      words,
      rules,
    }: {
      words: string[];
      rules: AicliPrivacyRule[];
    }) => aicliApi.setPrivacyCustom(words, rules),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: aicliKeys.privacy }),
  });
}

/**
 * 把后端的 aicli-data-updated 事件接到 react-query：按 kind 失效对应查询。
 * 在应用根部挂一次即可。
 */
export function useAicliEventBridge() {
  const queryClient = useQueryClient();
  useTauriEvent<string>(AICLI_EVENTS.dataUpdated, (kind) => {
    for (const queryKey of keysForDataKind(String(kind ?? ""))) {
      void queryClient.invalidateQueries({ queryKey });
    }
  });
}
