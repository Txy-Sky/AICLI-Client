import { useTranslation } from "react-i18next";
import { LayoutGrid, List } from "lucide-react";
import { SegmentedControl } from "@/components/ui/segmented-control";
import { useRememberedChoice } from "@/lib/rememberedChoice";

/** 发现栏的两种显示方式：紧凑列表（一行一条）和卡片网格（参考 magpie 的发现页） */
export type DiscoverView = "list" | "grid";
export const DISCOVER_VIEWS = ["list", "grid"] as const;

/** 本机记住的发现栏视图（localStorage，每个页面一个 key） */
export const DISCOVER_VIEW_KEYS = {
  skills: "cc-switch.skills.discoverView",
  mcp: "cc-switch.mcp.discoverView",
} as const;

export function useDiscoverView(
  key: (typeof DISCOVER_VIEW_KEYS)[keyof typeof DISCOVER_VIEW_KEYS],
  fallback: DiscoverView,
) {
  return useRememberedChoice<DiscoverView>(key, DISCOVER_VIEWS, fallback);
}

/** 列表 / 卡片切换：纯图标的小分段控件，文字只给读屏和悬停提示 */
export function DiscoverViewToggle({
  value,
  onValueChange,
}: {
  value: DiscoverView;
  onValueChange: (value: DiscoverView) => void;
}) {
  const { t } = useTranslation();
  return (
    <SegmentedControl<DiscoverView>
      aria-label={t("discoverView.aria")}
      size="sm"
      value={value}
      onValueChange={onValueChange}
      className="shrink-0"
      items={[
        {
          value: "list",
          icon: List,
          label: <span className="sr-only">{t("discoverView.list")}</span>,
          tip: t("discoverView.list"),
          className: "px-2",
        },
        {
          value: "grid",
          icon: LayoutGrid,
          label: <span className="sr-only">{t("discoverView.grid")}</span>,
          tip: t("discoverView.grid"),
          className: "px-2",
        },
      ]}
    />
  );
}
