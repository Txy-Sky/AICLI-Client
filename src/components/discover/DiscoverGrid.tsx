import * as React from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { cn } from "@/lib/utils";
import { DISCOVER_CARD_HEIGHT } from "./DiscoverCard";

/** 卡片最小宽度与间距（magpie：minmax(236px, 1fr)、gap 10） */
const MIN_COLUMN_WIDTH = 240;
const GAP = 12;

export function columnsFor(width: number): number {
  if (!(width > 0)) return 1;
  return Math.max(1, Math.floor((width + GAP) / (MIN_COLUMN_WIDTH + GAP)));
}

/**
 * 响应式卡片网格（按行虚拟化）：列数随容器宽度变（每列至少 240），
 * 只渲染可视区域附近的几行。仍是 <ul>/<li>，读屏照样按列表读、报总数和位置。
 */
export function DiscoverGrid<T>({
  items,
  getKey,
  renderCard,
  ariaLabel,
  className,
  footer,
}: {
  items: readonly T[];
  getKey: (item: T, index: number) => string;
  renderCard: (item: T, index: number) => React.ReactNode;
  ariaLabel: string;
  className?: string;
  /** 跟在网格后面、在同一个滚动区里（例如「加载更多」） */
  footer?: React.ReactNode;
}) {
  const scrollRef = React.useRef<HTMLDivElement>(null);
  const [width, setWidth] = React.useState(0);

  React.useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const update = () => setWidth(el.clientWidth);
    update();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(update);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const columns = columnsFor(width);
  const rowCount = Math.ceil(items.length / columns);
  const rowVirtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => DISCOVER_CARD_HEIGHT + GAP,
    overscan: 3,
  });

  const columnWidth = `calc((100% - ${GAP * (columns - 1)}px) / ${columns})`;

  return (
    <div
      ref={scrollRef}
      className={cn("min-h-0 overflow-auto scroll-stable", className)}
    >
      <ul
        aria-label={ariaLabel}
        className="relative m-0 list-none p-0"
        style={{ height: Math.max(0, rowVirtualizer.getTotalSize() - GAP) }}
      >
        {rowVirtualizer.getVirtualItems().flatMap((row) => {
          const cells: React.ReactNode[] = [];
          for (let col = 0; col < columns; col += 1) {
            const index = row.index * columns + col;
            if (index >= items.length) break;
            const item = items[index];
            cells.push(
              <li
                key={getKey(item, index)}
                aria-setsize={items.length}
                aria-posinset={index + 1}
                className="absolute top-0"
                style={{
                  transform: `translateY(${row.start}px)`,
                  width: columnWidth,
                  left:
                    col === 0
                      ? 0
                      : `calc(${columnWidth} * ${col} + ${GAP * col}px)`,
                  height: DISCOVER_CARD_HEIGHT,
                }}
              >
                {renderCard(item, index)}
              </li>,
            );
          }
          return cells;
        })}
      </ul>
      {footer}
    </div>
  );
}
