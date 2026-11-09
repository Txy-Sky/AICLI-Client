import { useQuery } from "@tanstack/react-query";
import { aicliApi } from "@/lib/api/aicli";
import { aicliKeys } from "./aicliKeys";

/** 服务端按天的用量汇总（近 30 天缓存；刷新完成由事件桥接失效）。 */
export function useAicliUsageSummary(enabled = true) {
  return useQuery({
    queryKey: aicliKeys.usageSummary,
    queryFn: () => aicliApi.getUsageSummary(),
    enabled,
    placeholderData: (previous) => previous,
  });
}

/**
 * 单个请求的服务端账单。billed_at 为空 = 尚未记账（界面显示「等待服务端
 * 记账」，不得用估算值冒充实扣）。
 */
export function useAicliRequestBill(requestId: string | null | undefined) {
  return useQuery({
    queryKey: aicliKeys.requestBill(requestId ?? ""),
    queryFn: () => aicliApi.getRequestBill(requestId as string),
    enabled: Boolean(requestId),
  });
}
