import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Check, Loader2, ShieldCheck, Zap } from "lucide-react";
import { Button } from "@/components/ui/button";
import { HelpTip } from "@/components/ui/help-tip";
import { HoverTip } from "@/components/ui/hover-tip";
import { Notice } from "@/components/ui/notice";
import { SegmentedControl } from "@/components/ui/segmented-control";
import { APP_DISPLAY_NAME } from "@/components/shell/AppGlyph";
import type { AppId } from "@/lib/api";
import { formatAicliQuota, readAccountQuota } from "@/lib/aicliQuota";
import { cn } from "@/lib/utils";
import {
  useAicliLines,
  useAicliProbeLines,
  useAicliSelectLine,
} from "@/hooks/useAicliLines";
import {
  useAicliCatalog,
  useAicliProfiles,
  useAicliRoutes,
  useAicliTakeoverStatus,
} from "@/hooks/useAicliProfiles";
import { useAicliUsageSummary } from "@/hooks/useAicliUsage";
import type {
  AicliAccountView,
  AicliCachedValue,
  AicliLine,
  AicliPrivacyView,
  AicliUsageSummary,
} from "@/types/aicli";
import { MiniBars, type MiniBar } from "./MiniBars";
import {
  aicliPresetDescription,
  aicliProfileName,
} from "./profiles/profileName";
import { takeoverDisplay } from "./takeover/useAicliTakeoverFlow";

export function formatSyncTime(seconds: number, locale?: string): string {
  return new Date(seconds * 1000).toLocaleString(locale);
}

type Range = 7 | 30;

interface AicliOverviewPanelProps {
  account: AicliAccountView;
  /** 服务端账户快照（缓存优先）；null = 从未取到。 */
  cached: AicliCachedValue | null | undefined;
  privacy?: AicliPrivacyView | null;
  onOpenProfiles: () => void;
  onOpenPrivacy?: () => void;
}

/** 最近 n 天（UTC 日期，新 → 旧的反序：旧 → 新），与后端 aicli_usage_daily 的 day 同口径。 */
function lastDays(n: number, now = new Date()): string[] {
  const out: string[] = [];
  for (let offset = n - 1; offset >= 0; offset -= 1) {
    const d = new Date(
      Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()),
    );
    d.setUTCDate(d.getUTCDate() - offset);
    out.push(d.toISOString().slice(0, 10));
  }
  return out;
}

function periodTotals(summary: AicliUsageSummary | undefined, days: string[]) {
  if (!summary) return null;
  const rows = summary.days.filter((row) => days.includes(row.day));
  if (rows.length === 0) return null;
  return {
    quota: rows.reduce((sum, row) => sum + row.quota, 0),
    requests: rows.reduce((sum, row) => sum + row.requests, 0),
  };
}

/**
 * 首页 AICLI 账户面板（F-10）：与原用量概览同一套视觉——一块面板、内部分隔线、
 * 指标格（说明在上、大号数字在下）。包含额度与使用条、服务端每日消耗趋势、线路
 * 健康与延迟、当前配置集摘要、隐私状态。状态规则沿用 F-03：缓存优先；同步失败显示
 * 缓存与时间并标「缓存数据」；没有快照写「暂无数据」，不写 0。
 */
export function AicliOverviewPanel({
  account,
  cached,
  privacy,
  onOpenProfiles,
  onOpenPrivacy,
}: AicliOverviewPanelProps) {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;
  const [range, setRange] = useState<Range>(7);
  const display = account.quota_display;
  const quota = readAccountQuota(cached?.value);
  const noData = t("aicli.account.noData");
  const fromCache = Boolean(cached && (cached.stale || cached.last_error));
  const fmt = (value: number | null | undefined) =>
    formatAicliQuota(value, display, locale);

  const summary = useAicliUsageSummary(true, 30).data;
  const days = lastDays(range);
  const totals = periodTotals(summary, days);

  const remaining = fmt(quota.remaining);
  const used = fmt(quota.used);
  const usedRatio =
    quota.remaining != null &&
    quota.used != null &&
    quota.remaining + quota.used > 0
      ? quota.used / (quota.remaining + quota.used)
      : null;

  const today = days[days.length - 1];
  const byDay = new Map((summary?.days ?? []).map((row) => [row.day, row]));
  const bars: MiniBar[] = days.map((day) => {
    const row = byDay.get(day);
    return {
      key: day,
      label: day,
      value: row ? row.quota : null,
      display: row
        ? `${fmt(row.quota) ?? noData} · ${t("aicli.overview.requestsValue", {
            count: row.requests,
          })}`
        : t("aicli.overview.dayNotSynced"),
      highlight: day === today,
      muted: row ? !row.is_final && day !== today : false,
    };
  });

  const initial = Array.from(account.username ?? "?")[0]?.toUpperCase();

  return (
    <section
      aria-labelledby="aicli-overview-title"
      data-testid="aicli-overview"
      className="overflow-hidden rounded-panel border border-border bg-surface"
    >
      {/* 头部：账户、用户组、来源与同步时间 */}
      <div
        data-testid="aicli-account-header"
        className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-border px-4 py-3"
      >
        <span
          aria-hidden="true"
          className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-[color-mix(in_srgb,var(--chart-usage)_16%,transparent)] text-strong font-semibold text-[var(--chart-usage)]"
        >
          {initial}
        </span>
        <div className="flex min-w-0 flex-col">
          <div className="flex min-w-0 items-center gap-2">
            <h2
              id="aicli-overview-title"
              className="m-0 truncate text-strong text-fg-1"
            >
              {account.username || noData}
            </h2>
            {account.user_group && (
              <span className="inline-flex h-5 items-center rounded-full bg-subtle px-2 text-badge text-fg-2">
                {account.user_group}
              </span>
            )}
            {fromCache && (
              <span className="inline-flex h-5 items-center rounded-full bg-warning-soft px-2 text-badge text-warning-text">
                {t("aicli.account.cached")}
              </span>
            )}
          </div>
          <span className="flex flex-wrap gap-x-1.5 text-caption text-fg-3">
            <span>{t("aicli.account.source")}</span>
            <span aria-hidden="true">·</span>
            <span>
              {cached
                ? t("aicli.account.lastSync", {
                    time: formatSyncTime(cached.fetched_at, locale),
                  })
                : t("aicli.account.neverSynced")}
            </span>
          </span>
        </div>
      </div>

      {/* 指标：剩余 / 已用 / 区间消耗 / 区间请求 */}
      <div className="grid grid-cols-2 divide-border border-b border-border md:grid-cols-4 md:divide-x [&>*:nth-child(-n+2)]:border-b [&>*:nth-child(-n+2)]:border-border md:[&>*:nth-child(-n+2)]:border-b-0">
        <Metric
          label={t("aicli.account.remaining")}
          help={t("aicli.overview.quotaHelp")}
          value={remaining ?? noData}
          testId="aicli-quota-remaining"
          strong
        >
          {usedRatio != null && (
            <div className="mt-2 flex flex-col gap-1">
              <div
                role="meter"
                aria-label={t("aicli.overview.usedShare")}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={Math.round(usedRatio * 100)}
                className="h-1.5 w-full overflow-hidden rounded-full bg-subtle"
              >
                <div
                  className="h-full rounded-full bg-[var(--chart-usage)]"
                  style={{ width: `${Math.max(usedRatio * 100, 1)}%` }}
                />
              </div>
              <span className="text-badge tabular-nums text-fg-3">
                {t("aicli.overview.usedPct", {
                  pct: Math.round(usedRatio * 100),
                })}
              </span>
            </div>
          )}
        </Metric>
        <Metric
          label={t("aicli.account.used")}
          value={used ?? noData}
          testId="aicli-quota-used"
        />
        <Metric
          label={t("aicli.overview.spend", { days: range })}
          value={totals ? (fmt(totals.quota) ?? noData) : noData}
          testId="aicli-server-spend"
        />
        <Metric
          label={t("aicli.overview.requests", { days: range })}
          value={totals ? totals.requests.toLocaleString(locale) : noData}
          testId="aicli-server-requests"
        />
      </div>

      {/* 服务端每日消耗 */}
      <div className="border-b border-border px-4 pb-3 pt-3">
        <div className="mb-2 flex items-center gap-2">
          <h3 className="m-0 text-caption font-semibold text-fg-2">
            {t("aicli.overview.dailyTitle")}
          </h3>
          <SegmentedControl<string>
            size="sm"
            className="ms-auto"
            aria-label={t("aicli.overview.rangeLabel")}
            value={String(range)}
            onValueChange={(value) => setRange(Number(value) as Range)}
            items={[
              { value: "7", label: t("aicli.overview.range7") },
              { value: "30", label: t("aicli.overview.range30") },
            ]}
          />
        </div>
        <MiniBars
          bars={bars}
          height={64}
          ariaLabel={t("aicli.overview.dailyTitle")}
          startLabel={days[0]?.slice(5)}
          endLabel={t("aicli.overview.today")}
        />
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-2 lg:divide-x lg:divide-border">
        <LinesTile />
        <ProfileTile account={account} onOpenProfiles={onOpenProfiles} />
      </div>

      {privacy?.privacy_enabled && (
        <PrivacyRow privacy={privacy} onOpenPrivacy={onOpenPrivacy} />
      )}
    </section>
  );
}

function Metric({
  label,
  help,
  value,
  testId,
  strong = false,
  children,
}: {
  label: string;
  help?: string;
  value: string;
  testId: string;
  strong?: boolean;
  children?: ReactNode;
}) {
  return (
    <div className="flex min-w-0 flex-col px-4 pb-3.5 pt-2.5">
      <span className="flex items-center gap-1 text-caption text-fg-2">
        {label}
        {help && <HelpTip title={label}>{help}</HelpTip>}
      </span>
      <HoverTip content={value}>
        <span
          data-testid={testId}
          className={cn(
            "truncate tabular-nums text-fg-1",
            strong
              ? "text-metric leading-8"
              : "text-[20px] font-semibold leading-8",
          )}
        >
          {value}
        </span>
      </HoverTip>
      {children}
    </div>
  );
}

/** 线路：健康圆点、延迟条、类型、当前 / 切换、测速。 */
function LinesTile() {
  const { t, i18n } = useTranslation();
  const lines = useAicliLines(true);
  const select = useAicliSelectLine();
  const probe = useAicliProbeLines();
  const view = lines.data;
  const lastChecked = Math.max(
    0,
    ...(view?.lines ?? []).map((line) => line.health?.checked_at ?? 0),
  );

  const health = (line: AicliLine) => {
    const h = line.health;
    if (!h || (h.status === "ok" && h.latency_ms == null))
      return { tone: "bg-fg-3", text: t("aicli.lines.untested"), ms: null };
    if (h.status !== "ok")
      return {
        tone: "bg-danger",
        text: t("aicli.lines.unreachable"),
        ms: null,
      };
    const ms = h.latency_ms ?? 0;
    return {
      tone: ms > 800 ? "bg-warning" : "bg-success",
      text: t("aicli.lines.latency", { ms }),
      ms,
    };
  };

  return (
    <section
      aria-labelledby="aicli-lines-title"
      className="flex min-w-0 flex-col gap-2 border-b border-border px-4 py-3 lg:border-b-0"
    >
      <div className="flex items-center gap-2">
        <h3
          id="aicli-lines-title"
          className="m-0 text-caption font-semibold text-fg-2"
        >
          {t("aicli.lines.title")}
        </h3>
        {lastChecked > 0 && (
          <span className="text-badge text-fg-3">
            {t("aicli.overview.checkedAt", {
              time: new Date(lastChecked * 1000).toLocaleTimeString(
                i18n.language,
                { hour: "2-digit", minute: "2-digit" },
              ),
            })}
          </span>
        )}
        <Button
          type="button"
          variant="quiet"
          size="compact"
          className="ms-auto"
          disabled={probe.isPending}
          onClick={() => probe.mutate()}
        >
          {probe.isPending ? (
            <Loader2 aria-hidden="true" className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <Zap aria-hidden="true" className="h-3.5 w-3.5" />
          )}
          {t("aicli.lines.probe")}
        </Button>
      </div>
      <ul className="m-0 flex list-none flex-col gap-1 p-0">
        {(view?.lines ?? []).map((line) => {
          const current = line.id === view?.selected_line_id;
          const h = health(line);
          return (
            <li
              key={line.id}
              data-testid={`aicli-line-${line.id}`}
              className={cn(
                "flex items-center gap-2.5 rounded-control px-2 py-1.5 text-body",
                current && "bg-selected",
              )}
            >
              <span
                aria-hidden="true"
                className={cn("h-2 w-2 shrink-0 rounded-full", h.tone)}
              />
              <span className="min-w-0 flex-1 truncate text-fg-1">
                {t(line.label_key)}
              </span>
              <span className="inline-flex h-5 items-center rounded-full border border-border px-1.5 text-badge text-fg-3">
                {t(`aicli.lines.transport.${line.transport}`, {
                  defaultValue: line.transport,
                })}
              </span>
              <span className="flex w-24 items-center gap-1.5">
                <span className="h-1 flex-1 overflow-hidden rounded-full bg-subtle">
                  {h.ms != null && (
                    <span
                      className={cn("block h-full rounded-full", h.tone)}
                      style={{ width: `${Math.min(h.ms / 10, 100)}%` }}
                    />
                  )}
                </span>
                <span className="w-14 text-end text-caption tabular-nums text-fg-2">
                  {h.text}
                </span>
              </span>
              {current ? (
                <span className="inline-flex w-14 items-center justify-end gap-1 text-caption font-medium text-fg-1">
                  <Check aria-hidden="true" className="h-3.5 w-3.5" />
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

/** 配置集：显示名与说明、模型数与分组、哪些客户端在用、需要修复。 */
function ProfileTile({
  account,
  onOpenProfiles,
}: {
  account: AicliAccountView;
  onOpenProfiles: () => void;
}) {
  const { t } = useTranslation();
  const routes = useAicliRoutes(true);
  const profiles = useAicliProfiles(true);
  const catalog = useAicliCatalog(true);
  const takeover = useAicliTakeoverStatus(true);
  const view = routes.data;
  const id =
    account.active_profile_id ?? view?.active_profile_id ?? "recommended";
  const profile = view?.profiles.find((entry) => entry.id === id);
  const name = aicliProfileName(t, profile, id);
  const description = aicliPresetDescription(t, id);
  const count = profile?.routes.length ?? 0;
  const groups = Array.from(
    new Set((profile?.routes ?? []).flatMap((route) => route.ordered_groups)),
  );
  const repair = profile?.repair;

  // 正在使用它的客户端：已接管，且没有绑定到别的配置集
  const bindings = profiles.data?.bindings ?? [];
  const usedBy = (takeover.data ?? [])
    .filter((entry) => takeoverDisplay(entry.state) === "active")
    .map((entry) => entry.app)
    .filter((app) => {
      const own = bindings.find(
        (binding) => binding.scope === "app" && binding.scope_key === app,
      );
      return (own?.profile_id ?? id) === id;
    });

  return (
    <section
      aria-labelledby="aicli-profile-title"
      className="flex min-w-0 flex-col gap-2 px-4 py-3"
    >
      <div className="flex items-center gap-2">
        <h3
          id="aicli-profile-title"
          className="m-0 text-caption font-semibold text-fg-2"
        >
          {t("aicli.profile.title")}
        </h3>
        <Button
          type="button"
          variant="quiet"
          size="compact"
          className="ms-auto"
          onClick={onOpenProfiles}
        >
          {t("aicli.profile.manage")}
        </Button>
      </div>
      <div className="flex items-baseline gap-2">
        <span className="truncate text-title text-fg-1">{name}</span>
        <span className="text-caption tabular-nums text-fg-3">
          {profile && count > 0
            ? t("aicli.profile.models", { count })
            : t("aicli.profile.noModels")}
        </span>
      </div>
      {description && (
        <p className="m-0 text-caption text-fg-2">{description}</p>
      )}
      {groups.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {groups.slice(0, 5).map((group) => {
            const info = catalog.data?.groups[group];
            const ratio =
              info?.ratio_known && info.ratio != null ? `×${info.ratio}` : null;
            return (
              <span
                key={group}
                className="inline-flex h-5 items-center gap-1 rounded-full bg-subtle px-2 text-badge text-fg-2"
              >
                {group}
                {ratio && (
                  <span className="tabular-nums text-fg-3">{ratio}</span>
                )}
              </span>
            );
          })}
          {groups.length > 5 && (
            <span className="inline-flex h-5 items-center rounded-full bg-subtle px-2 text-badge text-fg-3">
              +{groups.length - 5}
            </span>
          )}
        </div>
      )}
      {usedBy.length > 0 && (
        <p className="m-0 text-caption text-fg-3">
          {t("aicli.overview.usedBy", {
            apps: usedBy
              .map((app) => APP_DISPLAY_NAME[app as AppId] ?? app)
              .join("、"),
          })}
        </p>
      )}
      {repair?.needs_repair && (
        <Notice
          tone="warning"
          title={t("aicli.profile.needsRepair", {
            groups: repair.missing_groups.join("、"),
            count: repair.affected_models.length,
          })}
        />
      )}
    </section>
  );
}

/** 隐私开启时的引擎状态；可用性优先模式下发生降级时为红色提示（F-03）。 */
function PrivacyRow({
  privacy,
  onOpenPrivacy,
}: {
  privacy: AicliPrivacyView;
  onOpenPrivacy?: () => void;
}) {
  const { t } = useTranslation();
  const mode =
    privacy.privacy_failure_mode === "pass"
      ? t("aicli.privacy.modePass")
      : t("aicli.privacy.modeBlock");
  const engine = privacy.engine;
  const degraded = engine?.degraded ?? 0;
  return (
    <div className="flex flex-col gap-2 border-t border-border px-4 py-3">
      <div className="flex flex-wrap items-center gap-2 text-caption text-fg-2">
        <ShieldCheck
          aria-hidden="true"
          className="h-3.5 w-3.5 text-success-text"
        />
        <span>
          {t("aicli.privacy.enabled")} · {mode}
        </span>
        <span className="text-fg-3">
          {engine
            ? t("aicli.privacy.engineRunning", { count: engine.masked_values })
            : t("aicli.privacy.engineStopped")}
        </span>
        {onOpenPrivacy && (
          <Button
            type="button"
            variant="quiet"
            size="compact"
            className="ms-auto"
            onClick={onOpenPrivacy}
          >
            {t("aicli.overview.privacyOpen")}
          </Button>
        )}
      </div>
      {degraded > 0 && (
        <Notice
          tone="danger"
          title={t("aicli.privacy.degraded", { count: degraded })}
        />
      )}
    </div>
  );
}
