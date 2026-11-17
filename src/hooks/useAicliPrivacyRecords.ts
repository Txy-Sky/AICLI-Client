import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { aicliApi } from "@/lib/api/aicli";
import type { AicliPrivacyCategory } from "@/types/aicli";
import { aicliKeys } from "./aicliKeys";

/** 隐私页每页取多少条记录。 */
export const PRIVACY_EVENTS_PAGE = 50;

/** 最近 days 天的脱敏汇总（本机记录；页面打开时每 15 秒刷新一次）。 */
export function useAicliPrivacySummary(days: number, enabled = true) {
  return useQuery({
    queryKey: aicliKeys.privacySummary(days),
    queryFn: () => aicliApi.privacySummary(days),
    enabled,
    refetchInterval: 15_000,
    placeholderData: (previous) => previous,
  });
}

/** 记录列表：新 → 旧，按 id 翻页，可按类别筛选。 */
export function useAicliPrivacyEvents(
  days: number,
  category: AicliPrivacyCategory | null,
  enabled = true,
) {
  return useInfiniteQuery({
    queryKey: aicliKeys.privacyEvents(days, category),
    queryFn: ({ pageParam }) =>
      aicliApi.privacyEvents({
        days,
        category,
        beforeId: pageParam,
        limit: PRIVACY_EVENTS_PAGE,
      }),
    initialPageParam: null as number | null,
    getNextPageParam: (last) => last.next_before_id,
    enabled,
    refetchInterval: 15_000,
  });
}

/** 「试一试」：临时引擎预览，不影响真实请求。 */
export function useAicliPrivacyPreview() {
  return useMutation({
    mutationFn: (text: string) => aicliApi.privacyPreview(text),
  });
}

export function useAicliPrivacyClearEvents() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => aicliApi.privacyClearEvents(),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: aicliKeys.privacy }),
  });
}
