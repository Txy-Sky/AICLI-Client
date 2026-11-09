import { useQueryClient } from "@tanstack/react-query";
import { useSettings } from "@/hooks/useSettings";
import { usageKeys } from "@/lib/query/usage";
import type { SettingsFormState } from "@/hooks/useSettings";
import type { AppTypeFilter } from "@/types/usage";
import { UsageDashboard } from "./UsageDashboard";

interface UsagePageProps {
  /** 从应用页「查看此应用的用量」进入时带上的应用筛选 */
  initialAppType?: AppTypeFilter;
  /** 打开设置 → 本地路由（「记录请求用量」开关在那里） */
  onOpenRoutingSettings?: () => void;
  /** 标题旁的数据来源标注（F-03 首页用）。 */
  sourceNote?: string;
}

/** 侧栏「用量统计」全局页（v7 S6）：不开路由也有数据（读会话日志）。 */
export function UsagePage({
  initialAppType,
  onOpenRoutingSettings,
  sourceNote,
}: UsagePageProps = {}) {
  const { settings, autoSaveSettings } = useSettings();
  const queryClient = useQueryClient();

  const save = async (updates: Partial<SettingsFormState>) => {
    try {
      return (await autoSaveSettings(updates)) != null;
    } catch {
      // useSettings 已提示保存失败；将结果传回面板，回滚乐观更新的刷新间隔。
      return false;
    }
  };

  return (
    <UsageDashboard
      refreshIntervalMs={settings?.usageDashboardRefreshIntervalMs}
      onRefreshIntervalChange={(usageDashboardRefreshIntervalMs) =>
        save({ usageDashboardRefreshIntervalMs })
      }
      sessionAutoSyncEnabled={settings?.sessionAutoSyncEnabled ?? true}
      settingsReady={settings != null}
      onSessionAutoSyncEnabledChange={(sessionAutoSyncEnabled) =>
        save({ sessionAutoSyncEnabled })
      }
      initialAppType={initialAppType}
      onOpenRoutingSettings={onOpenRoutingSettings}
      usageDisplay={settings?.usageDisplay}
      onUsageDisplayChange={(usageDisplay) => save({ usageDisplay })}
      sourceNote={sourceNote}
      includeSessionLogs={settings?.usageIncludeSessionLogs ?? false}
      onIncludeSessionLogsChange={async (usageIncludeSessionLogs) => {
        // 后端按此设置过滤所有用量查询：保存后整页重新取数
        if (await save({ usageIncludeSessionLogs })) {
          await queryClient.invalidateQueries({ queryKey: usageKeys.all });
        }
      }}
    />
  );
}
