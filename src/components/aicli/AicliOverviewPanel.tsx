import { Fragment, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Check, ChevronRight, Loader2, Zap } from "lucide-react";
import { Button } from "@/components/ui/button";
import { HelpTip } from "@/components/ui/help-tip";
import { HoverTip } from "@/components/ui/hover-tip";
import { Notice } from "@/components/ui/notice";
import { SegmentedControl } from "@/components/ui/segmented-control";
import { APP_DISPLAY_NAME, AppGlyph } from "@/components/shell/AppGlyph";
import type { AppId } from "@/lib/api";
import { formatAicliQuota, readAccountQuota } from "@/lib/aicliQuota";
import { aicliApi, isAicliCommandError } from "@/lib/api/aicli";
import { proxyApi } from "@/lib/api/proxy";
import { proxyKeys, useProxyStatusQuery } from "@/lib/query/proxy";
import { aicliKeys } from "@/hooks/aicliKeys";
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
  AicliRouteProfile,
  AicliTakeoverStatus,
  AicliUsageSummary,
} from "@/types/aicli";
import { MiniBars, type MiniBar } from "./MiniBars";
import { useOpenAicliBind } from "./AicliBindGate";
import {
  aicliPresetDescription,
  aicliProfileName,
} from "./profiles/profileName";
import {
  AICLI_TAKEOVER_APPS,
  takeoverDisplay,
} from "./takeover/useAicliTakeoverFlow";

export function formatSyncTime(seconds: number, locale?: string): string {
  return new Date(seconds * 1000).toLocaleString(locale);
}

type Range = 7 | 30;

/** 请求链路上的环节；账户是终点，不可展开。 */
type FlowStep = "clients" | "proxy" | "privacy" | "line" | "profile";

type FlowTone = "ok" | "idle" | "warn" | "danger";

interface AicliOverviewPanelProps {
  account: AicliAccountView;
  /** 服务端账户快照（缓存优先）；null = 从未取到。 */
  cached: AicliCachedValue | null | undefined;
  privacy?: AicliPrivacyView | null;
  onOpenProfiles: () => void;
  onOpenPrivacy?: () => void;
  onOpenClients?: () => void;
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

function lineHealth(line: AicliLine | undefined) {
  const h = line?.health;
  if (!h || (h.status === "ok" && h.latency_ms == null))
    return { state: "untested" as const, ms: null };
  if (h.status !== "ok") return { state: "unreachable" as const, ms: null };
  return { state: "ok" as const, ms: h.latency_ms ?? 0 };
}

/**
 * 首页 AICLI 账户面板（F-10，2026-11 改版）：一块面板分三段。
 * 1. 头部：账户、用户组、来源与同步时间。
 * 2. 余额与每日消耗：剩余额度、使用条、按近期速度估算还能用几天；右边是服务端每日消耗柱状图。
 * 3. 请求链路：客户端 → 本地代理 → 隐私脱敏 → 线路 → 配置集 → 账户。每个环节显示当前状态，
 *    点开在下方就地查看和调整；有环节出问题时自动展开那一个，否则展开线路。
 * 状态规则沿用 F-03：缓存优先；同步失败显示缓存与时间并标「缓存数据」；没有快照写「暂无数据」，不写 0。
 */
export function AicliOverviewPanel({
  account,
  cached,
  privacy,
  onOpenProfiles,
  onOpenPrivacy,
  onOpenClients,
}: AicliOverviewPanelProps) {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;
  const [range, setRange] = useState<Range>(7);
  const [picked, setPicked] = useState<FlowStep | null>(null);
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
  // 按所选区间的日均消耗估算余额还能用几天；没有消耗或没有余额数据时不显示
  const runwayDays =
    totals && totals.quota > 0 && quota.remaining != null && quota.remaining > 0
      ? Math.floor(quota.remaining / (totals.quota / range))
      : null;

  const today = days[days.length - 1];
  const byDay = new Map((summary?.days ?? []).map((row) => [row.day, row]));
  const labelled = range === 7;
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
      // 一周的柱子够宽，直接标金额和日期；30 天太密，靠悬停
      caption: labelled ? (row ? (fmt(row.quota) ?? "") : "") : undefined,
      tick: labelled
        ? day === today
          ? t("aicli.overview.today")
          : day.slice(5)
        : undefined,
    };
  });

  const initial = Array.from(account.username ?? "?")[0]?.toUpperCase();

  // ── 请求链路各环节的状态 ──────────────────────────────────────────────
  const takeover = useAicliTakeoverStatus(true).data ?? [];
  const { data: proxyStatus } = useProxyStatusQuery();
  const linesView = useAicliLines(true).data;
  const routes = useAicliRoutes(true).data;

  const clientStates = AICLI_TAKEOVER_APPS.map((app) => ({
    app,
    display: takeoverDisplay(
      takeover.find((entry) => entry.app === app)?.state,
    ),
  }));
  const activeApps = clientStates.filter((entry) => entry.display === "active");
  const proxyDown =
    activeApps.length > 0 && proxyStatus !== undefined && !proxyStatus.running;

  const currentLine = linesView?.lines.find(
    (line) => line.id === linesView.selected_line_id,
  );
  const currentHealth = lineHealth(currentLine);

  const profileId =
    account.active_profile_id ?? routes?.active_profile_id ?? "recommended";
  const profile = routes?.profiles.find((entry) => entry.id === profileId);
  const modelCount = profile?.routes.length ?? 0;
  const needsRepair = Boolean(profile?.repair?.needs_repair);

  const privacyOn = privacy?.privacy_enabled ?? false;
  const degraded = privacy?.engine?.degraded ?? 0;

  // 有环节出问题时自动展开它；都正常时展开线路（最常调整的一项）
  const autoStep: FlowStep = proxyDown
    ? "proxy"
    : needsRepair
      ? "profile"
      : degraded > 0
        ? "privacy"
        : "line";
  const step = picked ?? autoStep;

  const syncTime = cached ? formatSyncTime(cached.fetched_at, locale) : null;
  const rebind = account.state === "needs_rebind";

  const nodes: {
    id: FlowStep;
    label: string;
    value: string;
    sub?: string;
    tone: FlowTone;
  }[] = [
    {
      id: "clients",
      label: t("nav.clients"),
      value: t("aicli.overview.flowClientsValue", {
        active: activeApps.length,
        total: clientStates.length,
      }),
      sub:
        activeApps.length > 0
          ? activeApps.map((entry) => APP_DISPLAY_NAME[entry.app]).join("、")
          : t("aicli.overview.flowClientsNone"),
      tone: activeApps.length > 0 ? "ok" : "idle",
    },
    {
      id: "proxy",
      label: t("aicli.overview.flowProxy"),
      value:
        proxyStatus === undefined
          ? t("aicli.overview.flowProxyUnknown")
          : proxyStatus.running
            ? t("aicli.overview.flowProxyRunning")
            : t("aicli.overview.flowProxyStopped"),
      tone: proxyDown ? "danger" : proxyStatus?.running ? "ok" : "idle",
    },
    {
      id: "privacy",
      label: t("nav.privacy"),
      value:
        privacy == null
          ? noData
          : privacyOn
            ? t("aicli.overview.flowPrivacyOn")
            : t("aicli.overview.flowPrivacyOff"),
      sub:
        privacy == null
          ? undefined
          : privacyOn
            ? privacy.engine
              ? t("aicli.privacy.engineRunning", {
                  count: privacy.engine.masked_values,
                })
              : t("aicli.privacy.engineStopped")
            : t("aicli.overview.flowPrivacyOffSub"),
      tone: degraded > 0 ? "danger" : privacyOn ? "ok" : "idle",
    },
    {
      id: "line",
      label: t("aicli.lines.title"),
      value: currentLine ? t(currentLine.label_key) : noData,
      sub: currentLine
        ? currentHealth.state === "ok"
          ? t("aicli.lines.latency", { ms: currentHealth.ms })
          : t(`aicli.lines.${currentHealth.state}`)
        : undefined,
      tone:
        currentHealth.state === "unreachable"
          ? "warn"
          : currentHealth.state === "ok"
            ? "ok"
            : "idle",
    },
    {
      id: "profile",
      label: t("aicli.profile.title"),
      value: aicliProfileName(t, profile, profileId),
      sub:
        profile && modelCount > 0
          ? t("aicli.profile.models", { count: modelCount })
          : t("aicli.profile.noModels"),
      tone: needsRepair ? "warn" : "ok",
    },
  ];

  return (
    <>
      <HomeAlert
        rebind={rebind}
        proxyDownApps={proxyDown ? activeApps.map((entry) => entry.app) : []}
        lines={linesView?.lines ?? []}
        currentLine={currentLine}
        stale={fromCache}
        syncTime={syncTime}
      />
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
            className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-action-soft text-strong font-semibold text-action-text"
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

        {/* 余额与每日消耗 */}
        <div className="grid grid-cols-1 gap-x-6 gap-y-4 border-b border-border px-4 py-4 lg:grid-cols-[minmax(0,1fr)_minmax(0,2fr)]">
          <div className="flex min-w-0 flex-col gap-2">
            <div className="flex min-w-0 flex-col">
              <span className="flex items-center gap-1 text-caption text-fg-2">
                {t("aicli.account.remaining")}
                <HelpTip title={t("aicli.account.remaining")}>
                  {t("aicli.overview.quotaHelp")}
                </HelpTip>
              </span>
              <HoverTip content={remaining ?? noData}>
                <span
                  data-testid="aicli-quota-remaining"
                  className="truncate text-metric leading-9 tabular-nums text-fg-1"
                >
                  {remaining ?? noData}
                </span>
              </HoverTip>
            </div>
            {usedRatio != null && (
              <div
                role="meter"
                aria-label={t("aicli.overview.usedShare")}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={Math.round(usedRatio * 100)}
                className="h-1.5 w-full overflow-hidden rounded-full bg-subtle"
              >
                <div
                  className="h-full rounded-full bg-action"
                  style={{ width: `${Math.max(usedRatio * 100, 1)}%` }}
                />
              </div>
            )}
            <span className="flex flex-wrap items-baseline gap-x-1.5 text-caption text-fg-2">
              <span>{t("aicli.account.used")}</span>
              <span
                data-testid="aicli-quota-used"
                className="font-medium tabular-nums text-fg-1"
              >
                {used ?? noData}
              </span>
              {usedRatio != null && (
                <span className="tabular-nums text-fg-3">
                  {t("aicli.overview.usedPct", {
                    pct: Math.round(usedRatio * 100),
                  })}
                </span>
              )}
            </span>
            {runwayDays != null && (
              <span
                data-testid="aicli-quota-runway"
                className="text-caption font-medium text-action-text"
              >
                {t("aicli.overview.runway", { days: range, count: runwayDays })}
              </span>
            )}
          </div>

          <div className="flex min-w-0 flex-col gap-2">
            <div className="flex flex-wrap items-center gap-x-4 gap-y-1">
              <h3 className="m-0 text-caption font-semibold text-fg-2">
                {t("aicli.overview.dailyTitle")}
              </h3>
              <InlineStat
                label={t("aicli.overview.spend", { days: range })}
                value={totals ? (fmt(totals.quota) ?? noData) : noData}
                testId="aicli-server-spend"
              />
              <InlineStat
                label={t("aicli.overview.requests", { days: range })}
                value={totals ? totals.requests.toLocaleString(locale) : noData}
                testId="aicli-server-requests"
              />
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
              height={labelled ? 112 : 72}
              ariaLabel={t("aicli.overview.dailyTitle")}
              startLabel={labelled ? undefined : days[0]?.slice(5)}
              endLabel={labelled ? undefined : t("aicli.overview.today")}
            />
          </div>
        </div>

        {/* 请求链路 */}
        <div className="flex flex-col gap-3 px-4 py-4">
          <div className="flex flex-wrap items-baseline gap-x-2.5 gap-y-0.5">
            <h3 className="m-0 text-caption font-semibold text-fg-2">
              {t("aicli.overview.flowTitle")}
            </h3>
            <span className="text-caption text-fg-3">
              {t("aicli.overview.flowHint")}
            </span>
          </div>
          <div className="flex flex-wrap items-stretch gap-1.5">
            {nodes.map((node) => (
              <Fragment key={node.id}>
                <FlowNode
                  label={node.label}
                  value={node.value}
                  sub={node.sub}
                  tone={node.tone}
                  selected={step === node.id}
                  testId={`aicli-flow-${node.id}`}
                  onSelect={() => setPicked(node.id)}
                />
                <span
                  aria-hidden="true"
                  className="flex items-center text-fg-3"
                >
                  <ChevronRight className="h-3.5 w-3.5" />
                </span>
              </Fragment>
            ))}
            <div
              data-testid="aicli-flow-account"
              data-tone={rebind ? "danger" : fromCache ? "stale" : "ok"}
              className={cn(
                "flex min-w-[124px] flex-1 flex-col gap-0.5 rounded-panel border px-3 py-2.5",
                rebind
                  ? "border-danger bg-danger-soft"
                  : fromCache
                    ? "border-dashed border-border-strong bg-surface"
                    : "border-transparent bg-action-soft",
              )}
            >
              <span
                className={cn(
                  "text-badge",
                  rebind || fromCache ? "text-fg-2" : "text-action-text",
                )}
              >
                {t("aicli.overview.flowAccount")}
              </span>
              <span
                className={cn(
                  "truncate text-strong font-semibold tabular-nums",
                  rebind ? "text-danger-text" : "text-fg-1",
                )}
              >
                {rebind
                  ? t("aicli.overview.flowAccountRebind")
                  : (remaining ?? noData)}
              </span>
              {rebind || fromCache
                ? syncTime && (
                    <span className="truncate text-badge font-normal text-fg-2">
                      {t("aicli.overview.flowAccountAsOf", { time: syncTime })}
                    </span>
                  )
                : usedRatio != null && (
                    <span className="text-badge font-normal tabular-nums text-fg-2">
                      {t("aicli.overview.usedPct", {
                        pct: Math.round(usedRatio * 100),
                      })}
                    </span>
                  )}
            </div>
          </div>

          <div
            data-testid={`aicli-flow-detail-${step}`}
            className="border-t border-border pt-3"
          >
            {step === "clients" && (
              <ClientsDetail
                states={clientStates}
                onOpenClients={onOpenClients}
              />
            )}
            {step === "proxy" && <ProxyDetail down={proxyDown} />}
            {step === "privacy" && (
              <PrivacyDetail privacy={privacy} onOpenPrivacy={onOpenPrivacy} />
            )}
            {step === "line" && <LinesDetail />}
            {step === "profile" && (
              <ProfileDetail
                profileId={profileId}
                profile={profile}
                takeover={takeover}
                onOpenProfiles={onOpenProfiles}
              />
            )}
          </div>
        </div>
      </section>
    </>
  );
}

/**
 * 首页顶部提示条（设计稿画板 18）：一次只显示最严重的一条，按钮直接解决问题。
 * 红：令牌失效、本地代理停了（已接管的客户端用不了）；橙：当前线路不通（要你决定换哪条）；
 * 灰：同步失败、显示的是缓存（只是告知）。
 */
function HomeAlert({
  rebind,
  proxyDownApps,
  lines,
  currentLine,
  stale,
  syncTime,
}: {
  rebind: boolean;
  proxyDownApps: AppId[];
  lines: AicliLine[];
  currentLine: AicliLine | undefined;
  stale: boolean;
  syncTime: string | null;
}) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const openBind = useOpenAicliBind();
  const select = useAicliSelectLine();
  const probe = useAicliProbeLines();
  const startProxy = useMutation({
    mutationFn: () => proxyApi.startProxyServer(),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: proxyKeys.status }),
  });
  const retry = useMutation({
    mutationFn: () => aicliApi.refresh("all"),
    onSettled: () => queryClient.invalidateQueries({ queryKey: aicliKeys.all }),
  });
  const failed = (failure: unknown) =>
    failure ? (
      <span role="alert" className="block text-danger-text">
        {t("aicli.overview.alertFailed", {
          message: isAicliCommandError(failure)
            ? failure.message
            : String(failure),
        })}
      </span>
    ) : null;
  const spinner = (pending: boolean) =>
    pending ? (
      <Loader2 aria-hidden="true" className="h-3.5 w-3.5 animate-spin" />
    ) : null;

  if (rebind) {
    return (
      <Notice
        tone="danger"
        className="border border-danger"
        title={
          <span className="font-semibold text-danger-text">
            {t("aicli.account.tokenInvalid")}
          </span>
        }
        actions={
          <Button
            type="button"
            variant="solid"
            size="compact"
            onClick={openBind}
          >
            {t("aicli.bind.rebindAction")}
          </Button>
        }
      >
        {t("aicli.overview.alertRebindBody")}
      </Notice>
    );
  }

  if (proxyDownApps.length > 0) {
    return (
      <Notice
        tone="danger"
        className="border border-danger"
        title={
          <span className="font-semibold text-danger-text">
            {t("aicli.overview.alertProxyTitle")}
          </span>
        }
        actions={
          <Button
            type="button"
            variant="solid"
            size="compact"
            disabled={startProxy.isPending}
            onClick={() => startProxy.mutate()}
          >
            {spinner(startProxy.isPending)}
            {t("aicli.overview.alertProxyAction")}
          </Button>
        }
      >
        {t("aicli.overview.alertProxyBody", {
          apps: proxyDownApps
            .map((app) => APP_DISPLAY_NAME[app] ?? app)
            .join("、"),
        })}
        {failed(startProxy.error)}
      </Notice>
    );
  }

  if (currentLine && lineHealth(currentLine).state === "unreachable") {
    // 测速正常的其他线路里最快的一条
    const fallback = lines
      .filter((line) => line.id !== currentLine.id)
      .map((line) => ({ line, health: lineHealth(line) }))
      .filter((entry) => entry.health.state === "ok")
      .sort((a, b) => (a.health.ms ?? 0) - (b.health.ms ?? 0))[0];
    const current = t(currentLine.label_key);
    return (
      <Notice
        tone="warning"
        className="border border-warning"
        title={
          <span className="font-semibold text-warning-text">
            {t("aicli.overview.alertLineTitle", { line: current })}
          </span>
        }
        actions={
          fallback ? (
            <Button
              type="button"
              variant="neutral"
              size="compact"
              disabled={select.isPending}
              onClick={() => select.mutate(fallback.line.id)}
            >
              {spinner(select.isPending)}
              {t("aicli.overview.alertLineSwitch", {
                line: t(fallback.line.label_key),
              })}
            </Button>
          ) : (
            <Button
              type="button"
              variant="neutral"
              size="compact"
              disabled={probe.isPending}
              onClick={() => probe.mutate()}
            >
              {spinner(probe.isPending)}
              {t("aicli.lines.probe")}
            </Button>
          )
        }
      >
        {fallback
          ? t("aicli.overview.alertLineBody", {
              line: t(fallback.line.label_key),
              ms: fallback.health.ms,
            })
          : t("aicli.overview.alertLineNoFallback")}
        {failed(select.error)}
      </Notice>
    );
  }

  if (stale) {
    return (
      <Notice
        tone="neutral"
        className="border border-border"
        title={
          <span className="font-semibold">
            {t("aicli.overview.alertStaleTitle")}
          </span>
        }
        actions={
          <Button
            type="button"
            variant="neutral"
            size="compact"
            disabled={retry.isPending}
            onClick={() => retry.mutate()}
          >
            {spinner(retry.isPending)}
            {t("aicli.overview.alertStaleAction")}
          </Button>
        }
      >
        {syncTime
          ? t("aicli.overview.alertStaleBody", { time: syncTime })
          : t("aicli.overview.alertStaleNoTime")}
        {failed(retry.error)}
      </Notice>
    );
  }

  return null;
}

function InlineStat({
  label,
  value,
  testId,
}: {
  label: string;
  value: string;
  testId: string;
}) {
  return (
    <span className="flex items-baseline gap-1.5 text-caption text-fg-2">
      {label}
      <span
        data-testid={testId}
        className="text-strong font-semibold tabular-nums text-fg-1"
      >
        {value}
      </span>
    </span>
  );
}

const TONE_DOT: Record<FlowTone, string> = {
  ok: "bg-action",
  idle: "bg-fg-3",
  warn: "bg-warning",
  danger: "bg-danger",
};

/** 链路上的一个环节：小标题带状态点、当前值、一行补充；选中时描边加粗。 */
function FlowNode({
  label,
  value,
  sub,
  tone,
  selected,
  testId,
  onSelect,
}: {
  label: string;
  value: string;
  sub?: string;
  tone: FlowTone;
  selected: boolean;
  testId: string;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      aria-pressed={selected}
      data-testid={testId}
      data-tone={tone}
      onClick={onSelect}
      className={cn(
        "flex min-w-[124px] flex-1 flex-col gap-0.5 rounded-panel border px-3 py-2.5 text-start transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        tone === "danger"
          ? "bg-danger-soft"
          : tone === "warn"
            ? "bg-warning-soft"
            : "bg-surface",
        selected
          ? "border-action ring-1 ring-action"
          : "border-border hover:border-border-strong",
      )}
    >
      <span className="flex items-center gap-1.5 text-badge text-fg-2">
        <span
          aria-hidden="true"
          className={cn("h-1.5 w-1.5 shrink-0 rounded-full", TONE_DOT[tone])}
        />
        {label}
      </span>
      <span className="truncate text-strong font-semibold text-fg-1">
        {value}
      </span>
      {sub && (
        <span className="truncate text-badge font-normal text-fg-3">{sub}</span>
      )}
    </button>
  );
}

/** 客户端：哪些已接管、哪些还按原配置用。 */
function ClientsDetail({
  states,
  onOpenClients,
}: {
  states: { app: AppId; display: ReturnType<typeof takeoverDisplay> }[];
  onOpenClients?: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col gap-2.5">
      <ul className="m-0 flex list-none flex-wrap gap-1.5 p-0">
        {states.map(({ app, display }) => (
          <li
            key={app}
            className={cn(
              "inline-flex h-7 items-center gap-1.5 rounded-full px-2.5 text-caption",
              display === "active"
                ? "bg-action-soft text-action-text"
                : "bg-subtle text-fg-2",
            )}
          >
            <AppGlyph
              app={app}
              size={14}
              badgeClassName={
                display === "active" ? "bg-action-soft" : "bg-subtle"
              }
            />
            {APP_DISPLAY_NAME[app]}
            <span aria-hidden="true">·</span>
            {t(`aicli.client.status.${display}`)}
          </li>
        ))}
      </ul>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
        <p className="m-0 min-w-0 flex-1 text-caption text-fg-2">
          {t("aicli.overview.flowClientsDetail")}
        </p>
        {onOpenClients && (
          <Button
            type="button"
            variant="neutral"
            size="compact"
            onClick={onOpenClients}
          >
            {t("aicli.overview.flowClientsOpen")}
          </Button>
        )}
      </div>
    </div>
  );
}

/** 本地代理：停了且有客户端在用时红色提示，否则一句说明。 */
function ProxyDetail({ down }: { down: boolean }) {
  const { t } = useTranslation();
  if (down) {
    return <Notice tone="danger" title={t("aicli.clients.proxyDown")} />;
  }
  return (
    <p className="m-0 text-caption text-fg-2">
      {t("aicli.overview.flowProxyDetail")}
    </p>
  );
}

/** 隐私：开启时显示出错处理方式，可用性优先模式下发生降级为红色提示（F-03）；未开启时说明后果。 */
function PrivacyDetail({
  privacy,
  onOpenPrivacy,
}: {
  privacy?: AicliPrivacyView | null;
  onOpenPrivacy?: () => void;
}) {
  const { t } = useTranslation();
  if (privacy == null) {
    return (
      <p className="m-0 text-caption text-fg-3">{t("aicli.account.noData")}</p>
    );
  }
  if (!privacy.privacy_enabled) {
    return (
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
        <p className="m-0 min-w-0 flex-1 text-caption text-fg-2">
          {t("aicli.overview.flowPrivacyOffDetail")}
        </p>
        {onOpenPrivacy && (
          <Button
            type="button"
            variant="neutral"
            size="compact"
            onClick={onOpenPrivacy}
          >
            {t("aicli.overview.flowPrivacyGo")}
          </Button>
        )}
      </div>
    );
  }
  const mode =
    privacy.privacy_failure_mode === "pass"
      ? t("aicli.privacy.modePass")
      : t("aicli.privacy.modeBlock");
  const degraded = privacy.engine?.degraded ?? 0;
  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
        <p className="m-0 min-w-0 flex-1 text-caption text-fg-2">
          {t("aicli.privacy.enabled")} · {mode}
        </p>
        {onOpenPrivacy && (
          <Button
            type="button"
            variant="quiet"
            size="compact"
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

/** 线路：健康圆点、延迟条、类型、当前 / 切换、测速。 */
function LinesDetail() {
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
    const h = lineHealth(line);
    if (h.state === "untested")
      return { tone: "bg-fg-3", text: t("aicli.lines.untested"), ms: null };
    if (h.state === "unreachable")
      return {
        tone: "bg-danger",
        text: t("aicli.lines.unreachable"),
        ms: null,
      };
    return {
      tone: h.ms > 800 ? "bg-warning" : "bg-success",
      text: t("aicli.lines.latency", { ms: h.ms }),
      ms: h.ms,
    };
  };

  return (
    <div className="flex min-w-0 flex-col gap-2">
      <div className="flex items-center gap-2">
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
    </div>
  );
}

/** 配置集：说明、分组与倍率、哪些客户端在用、需要修复。名称和模型数在链路节点上。 */
function ProfileDetail({
  profileId,
  profile,
  takeover,
  onOpenProfiles,
}: {
  profileId: string;
  profile: AicliRouteProfile | undefined;
  takeover: AicliTakeoverStatus[];
  onOpenProfiles: () => void;
}) {
  const { t } = useTranslation();
  const profiles = useAicliProfiles(true);
  const catalog = useAicliCatalog(true);
  const description = aicliPresetDescription(t, profileId);
  const groups = Array.from(
    new Set((profile?.routes ?? []).flatMap((route) => route.ordered_groups)),
  );
  const repair = profile?.repair;

  // 正在使用它的客户端：已接管，且没有绑定到别的配置集
  const bindings = profiles.data?.bindings ?? [];
  const usedBy = takeover
    .filter((entry) => takeoverDisplay(entry.state) === "active")
    .map((entry) => entry.app)
    .filter((app) => {
      const own = bindings.find(
        (binding) => binding.scope === "app" && binding.scope_key === app,
      );
      return (own?.profile_id ?? profileId) === profileId;
    });

  return (
    <div className="flex min-w-0 flex-col gap-2">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
        <p className="m-0 min-w-0 flex-1 text-caption text-fg-2">
          {description ??
            (usedBy.length > 0
              ? t("aicli.overview.usedBy", {
                  apps: usedBy
                    .map((app) => APP_DISPLAY_NAME[app as AppId] ?? app)
                    .join("、"),
                })
              : null)}
        </p>
        <Button
          type="button"
          variant="quiet"
          size="compact"
          onClick={onOpenProfiles}
        >
          {t("aicli.profile.manage")}
        </Button>
      </div>
      {groups.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {groups.slice(0, 8).map((group) => {
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
          {groups.length > 8 && (
            <span className="inline-flex h-5 items-center rounded-full bg-subtle px-2 text-badge text-fg-3">
              +{groups.length - 8}
            </span>
          )}
        </div>
      )}
      {description && usedBy.length > 0 && (
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
    </div>
  );
}
