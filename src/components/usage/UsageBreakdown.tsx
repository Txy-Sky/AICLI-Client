import { useTranslation } from "react-i18next";
import { AppGlyph, APP_DISPLAY_NAME } from "@/components/shell/AppGlyph";
import type { AppId } from "@/lib/api";
import { useModelStats, useUsageSummaryByApp } from "@/lib/query/usage";
import { useMoneyFormat } from "@/hooks/useMoneyFormat";
import type { UsageRangeSelection } from "@/types/usage";
import {
  fmtInt,
  getLocaleFromLanguage,
  getResolvedLang,
  parseFiniteNumber,
} from "./format";

/** 每张卡最多列几行；其余合并成一行「其他」。 */
const TOP = 5;

interface BreakdownRow {
  key: string;
  label: string;
  app?: AppId;
  cost: number;
  requests: number;
}

interface UsageBreakdownProps {
  range: UsageRangeSelection;
  /** 选了某个应用时只看它：「按客户端」一张卡没有意义，不显示 */
  appType?: string;
  providerName?: string;
  model?: string;
  refreshIntervalMs: number;
}

/**
 * 「按客户端」「按模型」两张拆分卡（2026-11 改版，设计稿画板 6）：所选范围内花在哪个工具、
 * 哪个模型上。按成本排序，成本都为 0（没配定价）时按请求数排；横条长度相对第一名。
 */
export function UsageBreakdown({
  range,
  appType,
  providerName,
  model,
  refreshIntervalMs,
}: UsageBreakdownProps) {
  const { t, i18n } = useTranslation();
  const { money } = useMoneyFormat();
  const locale = getLocaleFromLanguage(getResolvedLang(i18n));
  const refetch = {
    refetchInterval: refreshIntervalMs > 0 ? refreshIntervalMs : false,
  } as const;
  const byApp = useUsageSummaryByApp(
    range,
    { providerName, model },
    refetch,
  ).data;
  const byModel = useModelStats(
    range,
    { appType, providerName, model },
    refetch,
  ).data;

  const clientRows: BreakdownRow[] = (byApp ?? [])
    .filter((row) => row.summary.totalRequests > 0)
    .map((row) => ({
      key: row.appType,
      label: APP_DISPLAY_NAME[row.appType as AppId] ?? row.appType,
      app: row.appType in APP_DISPLAY_NAME ? (row.appType as AppId) : undefined,
      cost: parseFiniteNumber(row.summary.totalCost) ?? 0,
      requests: row.summary.totalRequests,
    }));
  const modelRows: BreakdownRow[] = (byModel ?? [])
    .filter((row) => row.requestCount > 0)
    .map((row) => ({
      key: row.model,
      label: row.model,
      cost: parseFiniteNumber(row.totalCost) ?? 0,
      requests: row.requestCount,
    }));

  const card = (
    id: string,
    title: string,
    rows: BreakdownRow[],
    testId: string,
  ) => {
    const byCost = rows.some((row) => row.cost > 0);
    const sorted = [...rows].sort((a, b) =>
      byCost ? b.cost - a.cost : b.requests - a.requests,
    );
    const shown = sorted.slice(0, TOP);
    const rest = sorted.slice(TOP);
    if (rest.length > 0) {
      shown.push({
        key: "__other",
        label: t("usage.breakdown.other", { count: rest.length }),
        cost: rest.reduce((sum, row) => sum + row.cost, 0),
        requests: rest.reduce((sum, row) => sum + row.requests, 0),
      });
    }
    const top = Math.max(
      0,
      ...shown.map((row) => (byCost ? row.cost : row.requests)),
    );
    return (
      <section
        aria-labelledby={id}
        data-testid={testId}
        className="flex h-full min-w-0 flex-col gap-3 rounded-panel border border-border bg-surface px-4 py-4"
      >
        <h3 id={id} className="m-0 text-section text-fg-1">
          {title}
        </h3>
        {shown.length === 0 ? (
          <p className="m-0 text-body text-fg-3">{t("usage.breakdown.none")}</p>
        ) : (
          <ul className="m-0 flex list-none flex-col gap-3 p-0">
            {shown.map((row) => {
              const value = byCost ? row.cost : row.requests;
              const pct = top > 0 ? Math.max(2, (value / top) * 100) : 0;
              const requests = t("usage.breakdown.requests", {
                count: row.requests,
                value: fmtInt(row.requests, locale),
              });
              return (
                <li
                  key={row.key}
                  data-testid={`${testId}-${row.key}`}
                  className="flex min-w-0 flex-col gap-1.5"
                >
                  <div className="flex min-w-0 items-baseline justify-between gap-3">
                    <span className="flex min-w-0 items-center gap-2 font-medium text-fg-1">
                      {row.app && (
                        <AppGlyph
                          app={row.app}
                          size={16}
                          badgeClassName="bg-surface"
                        />
                      )}
                      <span className="truncate">{row.label}</span>
                    </span>
                    <span className="shrink-0 text-body tabular-nums text-fg-2">
                      {byCost ? (
                        <>
                          <strong className="font-semibold text-fg-1">
                            {money(row.cost, 2)}
                          </strong>
                          {" · "}
                          {requests}
                        </>
                      ) : (
                        <strong className="font-semibold text-fg-1">
                          {requests}
                        </strong>
                      )}
                    </span>
                  </div>
                  <span
                    aria-hidden="true"
                    className="block h-2 overflow-hidden rounded-full bg-subtle"
                  >
                    <span
                      className="block h-full rounded-full bg-action"
                      style={{ width: `${pct}%` }}
                    />
                  </span>
                </li>
              );
            })}
          </ul>
        )}
      </section>
    );
  };

  return (
    <div className="grid shrink-0 grid-cols-1 gap-3 lg:grid-cols-2">
      {!appType &&
        card(
          "usage-breakdown-clients",
          t("usage.breakdown.byClient"),
          clientRows,
          "usage-breakdown-clients",
        )}
      <div className={appType ? "lg:col-span-2" : undefined}>
        {card(
          "usage-breakdown-models",
          t("usage.breakdown.byModel"),
          modelRows,
          "usage-breakdown-models",
        )}
      </div>
    </div>
  );
}
