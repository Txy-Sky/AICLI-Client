import * as React from "react";
import { cn } from "@/lib/utils";

/** 卡片固定高度：网格按行虚拟化，行高要事先知道 */
export const DISCOVER_CARD_HEIGHT = 148;

/**
 * 发现栏卡片（参考 magpie 的 .mk-card）：
 * 图标 + 名称 / 副行 → 两行描述 → 底部左侧附注、右侧操作。
 * 传 onOpen 时整张卡可点（名称是按钮，键盘可达）；底部操作区的点击不会冒泡成「打开」。
 */
export function DiscoverCard({
  logo,
  title,
  titleExtra,
  meta,
  description,
  descriptionTone = "default",
  descriptionTitle,
  descriptionId,
  footLeft,
  action,
  onOpen,
  openLabel,
  dimmed,
}: {
  logo: React.ReactNode;
  title: string;
  titleExtra?: React.ReactNode;
  meta?: React.ReactNode;
  description?: React.ReactNode;
  descriptionTone?: "default" | "danger";
  descriptionTitle?: string;
  descriptionId?: string;
  footLeft?: React.ReactNode;
  action?: React.ReactNode;
  onOpen?: () => void;
  openLabel?: string;
  /** 已添加 / 已安装：图标略淡 */
  dimmed?: boolean;
}) {
  return (
    <div
      onClick={onOpen}
      style={{ height: DISCOVER_CARD_HEIGHT }}
      className={cn(
        "flex min-w-0 flex-col gap-2 rounded-panel border border-border bg-surface p-3 transition-[border-color,box-shadow] duration-150",
        onOpen &&
          "cursor-pointer hover:border-border-strong hover:shadow-[0_6px_18px_-8px_rgba(0,0,0,0.18)]",
      )}
    >
      <div className="flex min-w-0 items-center gap-2.5">
        <span className={cn("shrink-0", dimmed && "opacity-90")}>{logo}</span>
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <div className="flex min-w-0 items-center gap-1.5">
            {onOpen ? (
              <button
                type="button"
                aria-label={openLabel}
                onClick={(event) => {
                  event.stopPropagation();
                  onOpen();
                }}
                className="min-w-0 truncate rounded-[4px] text-left text-body font-semibold text-fg-1 outline-none focus-visible:ring-1 focus-visible:ring-ring"
              >
                {title}
              </button>
            ) : (
              <span className="min-w-0 truncate text-body font-semibold text-fg-1">
                {title}
              </span>
            )}
            {titleExtra}
          </div>
          {meta && (
            <div className="flex min-w-0 items-center gap-1.5 text-caption text-fg-2">
              {meta}
            </div>
          )}
        </div>
      </div>
      <p
        id={descriptionId}
        title={descriptionTitle}
        className={cn(
          "m-0 line-clamp-2 min-h-[36px] text-caption",
          descriptionTone === "danger" ? "text-danger-text" : "text-fg-2",
        )}
      >
        {description}
      </p>
      <div className="mt-auto flex min-h-7 min-w-0 items-center gap-2">
        <div className="flex min-w-0 flex-1 items-center gap-1 text-caption text-fg-3">
          {footLeft}
        </div>
        {action && (
          <div
            className="flex min-w-0 max-w-[65%] shrink-0 items-center justify-end"
            onClick={(event) => event.stopPropagation()}
          >
            {action}
          </div>
        )}
      </div>
    </div>
  );
}
