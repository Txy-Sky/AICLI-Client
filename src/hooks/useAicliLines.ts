import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { aicliApi } from "@/lib/api/aicli";
import { aicliKeys } from "./aicliKeys";

/** 线路列表与健康状态（本地记录的最近一次探测）。 */
export function useAicliLines(enabled = true) {
  return useQuery({
    queryKey: aicliKeys.lines,
    queryFn: () => aicliApi.listLines(),
    enabled,
    placeholderData: (previous) => previous,
  });
}

/** 切换线路：只影响之后的请求；账户与路由视图都带当前线路，一并失效。 */
export function useAicliSelectLine() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => aicliApi.selectLine(id),
    onSettled: () =>
      Promise.all([
        queryClient.invalidateQueries({ queryKey: aicliKeys.lines }),
        queryClient.invalidateQueries({ queryKey: aicliKeys.account }),
        queryClient.invalidateQueries({ queryKey: aicliKeys.routes }),
      ]),
  });
}

/** 立即探测全部线路（后端按失败退避，未到时间的线路返回上次结果）。 */
export function useAicliProbeLines() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => aicliApi.probeLines(),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: aicliKeys.lines }),
  });
}
