import { HoverTip } from "@/components/ui/hover-tip";
import { cn } from "@/lib/utils";

export interface MiniBar {
  key: string;
  /** 悬停提示的标题（日期等）。 */
  label: string;
  /** null = 没有数据（画成空槽，不画成 0）。 */
  value: number | null;
  /** 悬停提示里的数值文案。 */
  display: string;
  /** 高亮（今天）。 */
  highlight?: boolean;
  /** 数据未定稿（今天、尚未同步完）时颜色浅一档。 */
  muted?: boolean;
  /** 柱子上方直接标的数值（柱子少时用；给了就每根都留出这一行，空串只占位）。 */
  caption?: string;
  /** 柱子下方的刻度（日期）；给了就不再需要 startLabel / endLabel。 */
  tick?: string;
}

interface MiniBarsProps {
  bars: MiniBar[];
  /** 柱区高度（px），含柱子上方的数值和下方的刻度。 */
  height?: number;
  /** 左下 / 右下的刻度文字（首日、末日）。 */
  startLabel?: string;
  endLabel?: string;
  ariaLabel: string;
  className?: string;
}

/**
 * 迷你柱状图（F-09 / F-10）：等宽柱、按最大值归一，悬停看精确值；没有数据的一天画成
 * 浅色空槽而不是 0 高度的柱，免得把「没取到」读成「没用」。
 * 柱子少（一周）时可以给每根柱子标数值和日期，不用悬停就能读。
 */
export function MiniBars({
  bars,
  height = 72,
  startLabel,
  endLabel,
  ariaLabel,
  className,
}: MiniBarsProps) {
  const max = Math.max(0, ...bars.map((bar) => bar.value ?? 0));
  const labelled = bars.some(
    (bar) => bar.caption !== undefined || bar.tick !== undefined,
  );
  return (
    <figure className={cn("m-0 flex flex-col gap-1.5", className)}>
      <div
        role="img"
        aria-label={ariaLabel}
        className={cn("flex items-end", labelled ? "gap-2" : "gap-[3px]")}
        style={{ height }}
      >
        {bars.map((bar) => {
          const ratio =
            bar.value == null || max <= 0 ? 0 : Math.max(bar.value / max, 0);
          const empty = bar.value == null;
          return (
            <HoverTip
              key={bar.key}
              content={
                <span className="flex flex-col">
                  <span className="text-fg-3">{bar.label}</span>
                  <span className="tabular-nums text-fg-1">{bar.display}</span>
                </span>
              }
            >
              <span
                data-testid={`mini-bar-${bar.key}`}
                data-empty={empty ? "true" : undefined}
                className="flex h-full min-w-0 flex-1 flex-col items-center gap-1"
              >
                {bar.caption !== undefined && (
                  <span
                    className={cn(
                      "h-4 max-w-full truncate text-badge tabular-nums",
                      bar.highlight
                        ? "font-semibold text-action-text"
                        : "font-normal text-fg-2",
                    )}
                  >
                    {bar.caption}
                  </span>
                )}
                <span className="flex min-h-0 w-full flex-1 items-end justify-center">
                  <span
                    className={cn(
                      "w-full rounded-t-[3px] transition-[height] duration-300 motion-reduce:transition-none",
                      labelled && "max-w-10",
                      empty
                        ? "h-full border border-dashed border-border bg-transparent"
                        : bar.highlight
                          ? "bg-[var(--chart-usage)]"
                          : bar.muted
                            ? "bg-[color-mix(in_srgb,var(--chart-usage)_35%,transparent)]"
                            : "bg-[color-mix(in_srgb,var(--chart-usage)_60%,transparent)]",
                    )}
                    style={
                      empty
                        ? undefined
                        : {
                            height: `${Math.max(ratio * 100, bar.value ? 4 : 2)}%`,
                          }
                    }
                  />
                </span>
                {bar.tick !== undefined && (
                  <span
                    className={cn(
                      "max-w-full truncate text-badge tabular-nums",
                      bar.highlight
                        ? "font-semibold text-fg-1"
                        : "font-normal text-fg-3",
                    )}
                  >
                    {bar.tick}
                  </span>
                )}
              </span>
            </HoverTip>
          );
        })}
      </div>
      {(startLabel || endLabel) && (
        <figcaption className="flex justify-between text-badge tabular-nums text-fg-3">
          <span>{startLabel}</span>
          <span>{endLabel}</span>
        </figcaption>
      )}
    </figure>
  );
}
