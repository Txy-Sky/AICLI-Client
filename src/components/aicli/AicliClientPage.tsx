import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { Loader2, RefreshCw } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { Notice } from "@/components/ui/notice";
import { APP_DISPLAY_NAME } from "@/components/shell/AppGlyph";
import type { AppId } from "@/lib/api";
import { aicliApi, isAicliCommandError } from "@/lib/api/aicli";
import { aicliKeys } from "@/hooks/aicliKeys";
import { useAicliAccount } from "@/hooks/useAicliAccount";
import {
  useAicliCatalog,
  useAicliProfiles,
  useAicliRoutes,
  useAicliTakeoverStatus,
} from "@/hooks/useAicliProfiles";
import { aicliProfileName } from "./profiles/profileName";
import { AicliProfilesPanel } from "./profiles/AicliProfilesPanel";
import { AicliTakeoverDialog } from "./takeover/AicliTakeoverDialog";
import { resolveClientProfile } from "./takeover/clientProfile";
import {
  AICLI_TAKEOVER_APPS,
  AICLI_TIER2_APPS,
  useAicliTakeoverFlow,
} from "./takeover/useAicliTakeoverFlow";
import type {
  AicliCatalog,
  AicliCompatibility,
  AicliModelRoute,
} from "@/types/aicli";

export {
  AICLI_TAKEOVER_APPS,
  AICLI_TIER2_APPS,
  takeoverDisplay,
  type AicliTakeoverDisplay,
} from "./takeover/useAicliTakeoverFlow";

/** 客户端走的线协议（后端 takeover 投影同一映射），决定看哪一列兼容级别。 */
export function protocolForApp(app: AppId): string {
  if (app === "claude" || app === "claude-desktop") return "anthropic";
  if (app === "gemini") return "gemini";
  // Tier 2（J 阶段）：aicli 提供方走 Chat Completions
  if (AICLI_TIER2_APPS.includes(app)) return "openai";
  return "openai-response";
}

function compatOf(
  catalog: AicliCatalog | null | undefined,
  route: AicliModelRoute,
  protocol: string,
): AicliCompatibility {
  return (
    catalog?.models[route.upstream_model]?.compat[protocol] ?? "unverified"
  );
}

/** 能力标签：服务端定价行里的 tags（逗号分隔，原样展示）。 */
function capabilityTags(
  catalog: AicliCatalog | null | undefined,
  route: AicliModelRoute,
): string[] {
  const pricing = catalog?.models[route.upstream_model]?.pricing;
  const tags =
    pricing && typeof pricing === "object"
      ? (pricing as { tags?: unknown }).tags
      : undefined;
  if (typeof tags !== "string") return [];
  return tags
    .split(",")
    .map((tag) => tag.trim())
    .filter(Boolean);
}

/** 兼容级别胶囊的颜色：完全兼容用品牌浅青，部分兼容用注意橙，其余灰。 */
const COMPAT_PILL: Record<AicliCompatibility, string> = {
  full: "bg-action-soft text-action-text",
  partial: "bg-warning-soft text-warning-text",
  text_only: "bg-subtle text-fg-2",
  unavailable: "bg-danger-soft text-danger-text",
  unverified: "bg-subtle text-fg-2",
};

type CompatFilter = "all" | "full" | "partial";

export interface AicliClientPageProps {
  app: AppId;
  /** 「在首页看整条链路」 */
  onOpenHome?: () => void;
  onOpenSessions?: () => void;
  /** 这个客户端有本机用量筛选时才传 */
  onOpenUsage?: () => void;
  onOpenConfigDir?: () => void;
  onOpenApps?: () => void;
}

/**
 * 客户端页（F-04，取代供应商列表；2026-11 改版，设计稿画板 7）：
 * - 顶部：接管状态胶囊与接管 / 停止接管按钮（确认对话框说明会还原配置）。
 * - 三张小卡：请求现在走哪里、用哪份配置集（可单独指定）、停止接管后回到什么。
 * - 本客户端可用模型表：模型、首选分组与倍率、兼容级别、能力标签；可按兼容级别筛选，可刷新模型目录。
 * - 底部：会话、用量、配置目录、安装与升级的入口。
 */
export function AicliClientPage({
  app,
  onOpenHome,
  onOpenSessions,
  onOpenUsage,
  onOpenConfigDir,
  onOpenApps,
}: AicliClientPageProps) {
  const { t, i18n } = useTranslation();
  const queryClient = useQueryClient();
  const appName = APP_DISPLAY_NAME[app] ?? app;
  const supported = AICLI_TAKEOVER_APPS.includes(app);

  const account = useAicliAccount().data;
  const bound = account?.state === "bound" || account?.state === "needs_rebind";
  const profiles = useAicliProfiles(bound);
  const routes = useAicliRoutes(bound);
  const catalog = useAicliCatalog(bound);
  const takeover = useAicliTakeoverStatus(bound && supported);
  const flow = useAicliTakeoverFlow(app, bound && supported);

  const [refreshError, setRefreshError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [filter, setFilter] = useState<CompatFilter>("all");
  const [profilesOpen, setProfilesOpen] = useState(false);

  const { display, conflict, busy } = flow;
  const error = flow.error ?? refreshError;
  const active = display === "active";

  // 本客户端的配置集：客户端专用绑定 > 账户默认 > 推荐预设
  const { id: profileId, profile } = resolveClientProfile(
    app,
    account,
    profiles.data,
    routes.data,
  );
  const profileName = aicliProfileName(t, profile, profileId);
  const ownProfile = Boolean(
    profiles.data?.bindings.some(
      (binding) => binding.scope === "app" && binding.scope_key === app,
    ),
  );
  const lineId = routes.data?.selected_line_id ?? account?.selected_line_id;
  const lineName = lineId ? t(`aicli.lines.${lineId}`) : "";
  const protocol = protocolForApp(app);
  const models = profile?.routes ?? [];
  const compatOfRoute = (route: AicliModelRoute) =>
    compatOf(catalog.data, route, protocol);
  const fullCount = models.filter((r) => compatOfRoute(r) === "full").length;
  const partialCount = models.filter(
    (route) => compatOfRoute(route) === "partial",
  ).length;
  const shown =
    filter === "all"
      ? models
      : models.filter((route) => compatOfRoute(route) === filter);
  const takenAt = takeover.data?.find(
    (entry) => entry.app === app,
  )?.taken_over_at;

  const groupText = (route: AicliModelRoute) => {
    const [first, ...rest] = route.ordered_groups;
    if (!first) return "—";
    const info = catalog.data?.groups[first];
    const ratio =
      info && info.ratio_known && info.ratio != null ? ` ×${info.ratio}` : "";
    return rest.length > 0
      ? t("aicli.client.groupWithFallback", {
          group: `${first}${ratio}`,
          count: rest.length,
        })
      : `${first}${ratio}`;
  };

  const refreshCatalog = async () => {
    setRefreshing(true);
    setRefreshError(null);
    try {
      await aicliApi.refresh("all");
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: aicliKeys.catalog }),
        queryClient.invalidateQueries({ queryKey: aicliKeys.routes }),
      ]);
    } catch (failure) {
      setRefreshError(
        t("aicli.client.failed", {
          message: isAicliCommandError(failure)
            ? failure.message
            : String(failure),
        }),
      );
    } finally {
      setRefreshing(false);
    }
  };

  const statusLabel = conflict
    ? t("aicli.client.status.conflict")
    : t(`aicli.client.status.${display}`);

  const card =
    "flex min-w-0 flex-col gap-1 rounded-panel border border-border bg-surface px-4 py-3";
  const cardLabel = "text-caption text-fg-2";
  const cardValue = "truncate text-title text-fg-1";
  const link =
    "self-start text-caption font-medium text-action-text hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring rounded-sm";

  const chip = (value: CompatFilter, label: string) => (
    <button
      key={value}
      type="button"
      aria-pressed={filter === value}
      onClick={() => setFilter(value)}
      className={cn(
        "inline-flex h-7 items-center rounded-full px-3 text-caption transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        filter === value
          ? "bg-action-soft font-semibold text-action-text"
          : "border border-border-strong bg-surface text-fg-1 hover:bg-subtle",
      )}
    >
      {label}
    </button>
  );

  const links = [
    onOpenSessions && {
      key: "sessions",
      label: t("aicli.client.openSessions"),
      onClick: onOpenSessions,
    },
    onOpenUsage && {
      key: "usage",
      label: t("aicli.client.openUsage"),
      onClick: onOpenUsage,
    },
    onOpenConfigDir && {
      key: "config",
      label: t("appPage.configDirectory"),
      onClick: onOpenConfigDir,
    },
    onOpenApps && {
      key: "apps",
      label: t("appPage.installAndUpgrade"),
      onClick: onOpenApps,
    },
  ].filter(Boolean) as { key: string; label: string; onClick: () => void }[];

  return (
    <div
      id="main-content"
      data-testid="aicli-client-page"
      className="min-h-0 flex-1 overflow-y-auto scroll-stable px-6 pb-12 pt-4"
    >
      <div className="flex flex-col gap-5">
        {!bound && account && (
          <Notice tone="neutral" title={t("aicli.client.notBound")} />
        )}

        <section
          aria-labelledby="aicli-client-takeover"
          className="flex flex-col gap-3"
        >
          <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
            <h2
              id="aicli-client-takeover"
              className="m-0 text-section text-fg-1"
            >
              {t("aicli.client.takeoverTitle")}
            </h2>
            {supported && bound && (
              <span
                data-testid="aicli-takeover-state"
                className={cn(
                  "inline-flex h-6 items-center rounded-full px-2.5 text-caption font-medium",
                  conflict
                    ? "bg-warning-soft text-warning-text"
                    : active
                      ? "bg-action-soft text-action-text"
                      : "bg-subtle text-fg-2",
                )}
              >
                {statusLabel}
              </span>
            )}
            {supported && bound && (
              <div className="ms-auto flex items-center gap-2">
                {active ? (
                  <Button
                    type="button"
                    variant="neutral"
                    size="regular"
                    disabled={busy}
                    onClick={() => flow.request("release")}
                  >
                    {t("aicli.client.release")}
                  </Button>
                ) : (
                  <Button
                    type="button"
                    variant="solid"
                    size="regular"
                    disabled={busy || display === "pending"}
                    onClick={() => flow.request("takeover")}
                  >
                    {t("aicli.client.takeOver")}
                  </Button>
                )}
              </div>
            )}
          </div>
          {!supported && (
            <p className="m-0 text-body text-fg-2">
              {t("aicli.client.unsupported", { app: appName })}
            </p>
          )}
          {conflict && (
            <Notice
              tone="warning"
              title={t("aicli.client.conflictNotice", { app: appName })}
            />
          )}
          {error && (
            <p role="alert" className="m-0 text-caption text-danger-text">
              {error}
            </p>
          )}

          {bound && supported && (
            <div className="grid grid-cols-1 gap-3 md:grid-cols-3">
              <div className={card} data-testid="aicli-client-route">
                <span className={cardLabel}>
                  {t("aicli.client.routeLabel")}
                </span>
                <span className={cardValue}>
                  {active
                    ? t("aicli.client.routeActive", { line: lineName })
                    : t("aicli.client.routeInactive")}
                </span>
                {onOpenHome && (
                  <button type="button" className={link} onClick={onOpenHome}>
                    {t("aicli.client.routeOpenHome")}
                  </button>
                )}
              </div>
              <div className={card} data-testid="aicli-client-profile">
                <span className={cardLabel}>
                  {t("aicli.client.profileLabel")}
                </span>
                <span className="flex min-w-0 flex-wrap items-baseline gap-x-2">
                  <span className={cardValue}>{profileName}</span>
                  <span className="text-caption text-fg-3">
                    {t(
                      ownProfile
                        ? "aicli.client.profileOwn"
                        : "aicli.client.profileFollow",
                      { count: models.length },
                    )}
                  </span>
                </span>
                <button
                  type="button"
                  className={link}
                  onClick={() => setProfilesOpen(true)}
                >
                  {ownProfile
                    ? t("aicli.client.profileChange")
                    : t("aicli.client.profilePick")}
                </button>
              </div>
              <div className={card} data-testid="aicli-client-restore">
                <span className={cardLabel}>
                  {t("aicli.client.restoreLabel")}
                </span>
                <span className={cardValue}>
                  {active
                    ? t("aicli.clients.restoreTo")
                    : t("aicli.client.restoreNow")}
                </span>
                <span className="text-caption text-fg-3">
                  {active && takenAt != null
                    ? t("aicli.clients.restoreBackedUp", {
                        time: new Date(takenAt * 1000).toLocaleString(
                          i18n.language,
                        ),
                      })
                    : t("aicli.client.restoreHint")}
                </span>
              </div>
            </div>
          )}
        </section>

        {bound && (
          <section
            aria-labelledby="aicli-client-models"
            className="flex flex-col gap-3"
          >
            <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
              <div className="flex min-w-0 flex-wrap items-baseline gap-x-2.5">
                <h2
                  id="aicli-client-models"
                  className="m-0 text-section text-fg-1"
                >
                  {t("aicli.client.modelsTitle")}
                </h2>
                <span className="text-caption text-fg-3">
                  {t("aicli.client.modelsHint", { app: appName })}
                </span>
              </div>
              <div className="ms-auto flex flex-wrap items-center gap-2">
                {models.length > 0 && (
                  <>
                    {chip(
                      "all",
                      t("aicli.client.filterAll", { count: models.length }),
                    )}
                    {fullCount > 0 &&
                      chip(
                        "full",
                        t("aicli.client.filterCompat", {
                          label: t("aicli.client.compat.full"),
                          count: fullCount,
                        }),
                      )}
                    {partialCount > 0 &&
                      chip(
                        "partial",
                        t("aicli.client.filterCompat", {
                          label: t("aicli.client.compat.partial"),
                          count: partialCount,
                        }),
                      )}
                  </>
                )}
                <Button
                  type="button"
                  variant="quiet"
                  size="compact"
                  disabled={refreshing}
                  onClick={() => void refreshCatalog()}
                >
                  {refreshing ? (
                    <Loader2
                      aria-hidden="true"
                      className="h-3.5 w-3.5 animate-spin"
                    />
                  ) : (
                    <RefreshCw aria-hidden="true" className="h-3.5 w-3.5" />
                  )}
                  {t("aicli.client.refreshCatalog")}
                </Button>
              </div>
            </div>
            {app === "claude" && partialCount > 0 && (
              <Notice
                tone="warning"
                title={t("aicli.client.partialNotice", { count: partialCount })}
              />
            )}
            {models.length === 0 ? (
              <p className="m-0 rounded-panel border border-dashed border-border px-4 py-6 text-center text-body text-fg-2">
                {t("aicli.client.noModels")}
              </p>
            ) : (
              <div className="overflow-x-auto rounded-panel border border-border bg-surface">
                <table className="w-full min-w-[560px] border-collapse text-body">
                  <thead>
                    <tr className="text-caption text-fg-2">
                      <th className="border-b border-border px-4 py-2 text-start font-medium">
                        {t("aicli.client.columns.model")}
                      </th>
                      <th className="border-b border-border px-3 py-2 text-start font-medium">
                        {t("aicli.client.columns.group")}
                      </th>
                      <th className="border-b border-border px-3 py-2 text-start font-medium">
                        {t("aicli.client.columns.compat")}
                      </th>
                      <th className="border-b border-border px-4 py-2 text-start font-medium">
                        {t("aicli.client.columns.capabilities")}
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    {shown.map((route) => {
                      const compat = compatOfRoute(route);
                      const tags = capabilityTags(catalog.data, route);
                      return (
                        <tr
                          key={route.public_model}
                          data-testid={`aicli-model-${route.public_model}`}
                          className="border-b border-border last:border-b-0"
                        >
                          <td className="px-4 py-2.5 font-medium text-fg-1">
                            {route.public_model}
                          </td>
                          <td className="px-3 py-2.5 tabular-nums text-fg-2">
                            {groupText(route)}
                          </td>
                          <td className="px-3 py-2.5">
                            <span
                              className={cn(
                                "inline-flex h-5 items-center rounded-full px-2 text-badge font-medium",
                                COMPAT_PILL[compat],
                              )}
                            >
                              {t(`aicli.client.compat.${compat}`)}
                            </span>
                          </td>
                          <td className="px-4 py-2.5 text-fg-2">
                            <span className="flex flex-wrap gap-1">
                              {tags.map((tag) => (
                                <Badge
                                  key={tag}
                                  variant="outline"
                                  className="h-5 px-1.5 text-badge"
                                >
                                  {tag}
                                </Badge>
                              ))}
                            </span>
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            )}
          </section>
        )}

        {links.length > 0 && (
          <div className="flex flex-wrap gap-2">
            {links.map((item) => (
              <Button
                key={item.key}
                type="button"
                variant="neutral"
                size="regular"
                onClick={item.onClick}
              >
                {item.label}
              </Button>
            ))}
          </div>
        )}
      </div>

      <AicliTakeoverDialog flow={flow} />
      <AicliProfilesPanel
        isOpen={profilesOpen}
        onClose={() => setProfilesOpen(false)}
      />
    </div>
  );
}
