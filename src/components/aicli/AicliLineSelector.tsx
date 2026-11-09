import { useTranslation } from "react-i18next";
import { Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import {
  useAicliLines,
  useAicliProbeLines,
  useAicliSelectLine,
} from "@/hooks/useAicliLines";
import type { AicliLine } from "@/types/aicli";

/**
 * 线路块（F-03）：当前线路、延迟、类型与切换。切换只影响之后的请求。
 */
export function AicliLineSelector({ enabled = true }: { enabled?: boolean }) {
  const { t } = useTranslation();
  const lines = useAicliLines(enabled);
  const select = useAicliSelectLine();
  const probe = useAicliProbeLines();
  const view = lines.data;

  const latencyText = (line: AicliLine): string => {
    const health = line.health;
    if (!health) return t("aicli.lines.untested");
    if (health.status !== "ok") return t("aicli.lines.unreachable");
    return health.latency_ms == null
      ? t("aicli.lines.untested")
      : t("aicli.lines.latency", { ms: health.latency_ms });
  };

  return (
    <section
      aria-labelledby="aicli-lines-title"
      className="flex min-w-0 flex-col gap-2 rounded-panel border border-border bg-surface p-4"
    >
      <div className="flex items-center gap-2">
        <h2 id="aicli-lines-title" className="m-0 text-section text-fg-1">
          {t("aicli.lines.title")}
        </h2>
        <Button
          type="button"
          variant="quiet"
          size="compact"
          className="ms-auto"
          disabled={probe.isPending || !enabled}
          onClick={() => probe.mutate()}
        >
          {probe.isPending && (
            <Loader2 aria-hidden="true" className="h-3.5 w-3.5 animate-spin" />
          )}
          {t("aicli.lines.probe")}
        </Button>
      </div>
      <ul className="m-0 flex list-none flex-col gap-1 p-0">
        {(view?.lines ?? []).map((line) => {
          const current = line.id === view?.selected_line_id;
          return (
            <li
              key={line.id}
              data-testid={`aicli-line-${line.id}`}
              className={cn(
                "flex items-center gap-2 rounded-control px-2 py-1.5 text-body",
                current && "bg-selected",
              )}
            >
              <span className="min-w-0 flex-1 truncate text-fg-1">
                {t(line.label_key)}
              </span>
              <span className="text-caption text-fg-3">
                {t(`aicli.lines.transport.${line.transport}`, {
                  defaultValue: line.transport,
                })}
              </span>
              <span className="w-16 text-end text-caption tabular-nums text-fg-2">
                {latencyText(line)}
              </span>
              {current ? (
                <span className="w-14 text-end text-caption font-medium text-fg-1">
                  {t("aicli.lines.current")}
                </span>
              ) : (
                <Button
                  type="button"
                  variant="neutral"
                  size="compact"
                  className="w-14"
                  disabled={select.isPending}
                  onClick={() => select.mutate(line.id)}
                >
                  {t("aicli.lines.switch")}
                </Button>
              )}
            </li>
          );
        })}
      </ul>
      <p className="m-0 text-caption text-fg-3">
        {t("aicli.lines.switchNote")}
      </p>
    </section>
  );
}
