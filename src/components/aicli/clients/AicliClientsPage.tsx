import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowRight,
  Download,
  Info,
  Loader2,
  MonitorSmartphone,
} from "lucide-react";
import { AppPageHeader } from "@/components/shell/AppPageHeader";
import { APP_DISPLAY_NAME, AppGlyph } from "@/components/shell/AppGlyph";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { HelpTip } from "@/components/ui/help-tip";
import { Notice } from "@/components/ui/notice";
import { Switch } from "@/components/ui/switch";
import { orderAppIds } from "@/config/appConfig";
import type { AppId } from "@/lib/api";
import { aicliErrorCode } from "@/lib/api/aicli";
import { cn } from "@/lib/utils";
import { useProxyStatusQuery } from "@/lib/query/proxy";
import { useAicliAccount } from "@/hooks/useAicliAccount";
import {
  useAicliProfiles,
  useAicliRoutes,
  useAicliTakeOver,
  useAicliTakeoverStatus,
} from "@/hooks/useAicliProfiles";
import type { VisibleApps } from "@/types";
import { aicliProfileName } from "../profiles/profileName";
import { AicliTakeoverDialog } from "../takeover/AicliTakeoverDialog";
import { resolveClientProfile } from "../takeover/clientProfile";
import {
  AICLI_TAKEOVER_APPS,
  takeoverDisplay,
  useAicliTakeoverFlow,
} from "../takeover/useAicliTakeoverFlow";

/** 有自己专属页（工作区、配置、记忆）的客户端：不能接管时单列一行，页面仍可进入；
 *  能接管时和其他客户端一样在列表里，点名字进入同一个页面。 */
const DEDICATED_PAGE_APPS: readonly AppId[] = ["openclaw", "hermes"];

/** 列表四列：客户端 / 请求现在走哪里 / 停止接管后回到 / 开关。表头和每一行共用。 */
const ROW_GRID =
  "grid grid-cols-[minmax(0,1.1fr)_minmax(0,1.7fr)_minmax(0,1.1fr)_112px] items-center gap-4";

interface BulkResult {
  conflicts: AppId[];
  failed: AppId[];
}

interface AicliClientsPageProps {
  visibleApps: VisibleApps;
  appOrder?: string[];
  onOpenApp: (app: AppId) => void;
  onOpenApps: () => void;
}

/**
 * 客户端总览（F-08，2026-11 改版）：侧栏只有一个「客户端」入口，进来是一张列表。
 * 支持接管的客户端一行一个，三列分别回答「请求现在走哪里」「停止接管后回到哪里」，
 * 行尾一个开关（开 = 经 AICLI）；打开或关闭都先过确认对话框，停止接管会还原为接管前的
 * 配置。点行里的名字进入详情页。顶部一条分段条显示接管了几个，可以一次接管其余的。
 * 有专属页的客户端单列一行，其余客户端列为「即将支持」。
 */
export function AicliClientsPage({
  visibleApps,
  appOrder,
  onOpenApp,
  onOpenApps,
}: AicliClientsPageProps) {
  const { t, i18n } = useTranslation();
  const account = useAicliAccount().data;
  const bound = account?.state === "bound" || account?.state === "needs_rebind";
  const takeover = useAicliTakeoverStatus(bound);
  const { data: proxyStatus } = useProxyStatusQuery();
  const [bulkResult, setBulkResult] = useState<BulkResult | null>(null);

  const ordered = orderAppIds(appOrder).filter((app) => visibleApps[app]);
  const supported = ordered.filter((app) => AICLI_TAKEOVER_APPS.includes(app));
  const dedicated = ordered.filter(
    (app) =>
      DEDICATED_PAGE_APPS.includes(app) && !AICLI_TAKEOVER_APPS.includes(app),
  );
  const comingSoon = ordered.filter(
    (app) =>
      !AICLI_TAKEOVER_APPS.includes(app) && !DEDICATED_PAGE_APPS.includes(app),
  );

  const entryOf = (app: AppId) =>
    takeover.data?.find((entry) => entry.app === app);
  const displayOf = (app: AppId) => takeoverDisplay(entryOf(app)?.state);
  const activeCount = supported.filter(
    (app) => displayOf(app) === "active",
  ).length;
  const rest = supported.filter((app) => displayOf(app) === "inactive");
  const proxyDown =
    activeCount > 0 && proxyStatus !== undefined && !proxyStatus.running;
  const names = (apps: AppId[]) =>
    apps.map((app) => APP_DISPLAY_NAME[app]).join("、");

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <AppPageHeader
        icon={<MonitorSmartphone className="h-5 w-5" strokeWidth={1.5} />}
        title={t("aicli.clients.title")}
        titleExtra={
          <HelpTip title={t("aicli.clients.title")}>
            {t("aicli.clients.help")}
          </HelpTip>
        }
        actions={
          <Button
            type="button"
            variant="neutral"
            size="compact"
            onClick={onOpenApps}
          >
            <Download aria-hidden="true" className="h-3.5 w-3.5" />
            {t("aicli.clients.installApps")}
          </Button>
        }
      />
      <div
        id="main-content"
        data-testid="aicli-clients-page"
        className="flex min-h-0 flex-1 flex-col gap-6 overflow-y-auto scroll-stable px-6 pb-10 pt-4"
      >
        <div className="flex flex-wrap items-end justify-between gap-x-4 gap-y-3">
          <div className="flex min-w-0 flex-col gap-2">
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
              <span
                role="img"
                aria-label={t("aicli.clients.summaryAria", {
                  active: activeCount,
                  total: supported.length,
                })}
                className="flex gap-1"
              >
                {supported.map((app) => (
                  <span
                    key={app}
                    className={cn(
                      "h-2 w-6 rounded-full",
                      displayOf(app) === "active"
                        ? "bg-action"
                        : "bg-border-strong",
                    )}
                  />
                ))}
              </span>
              <span className="text-body text-fg-1">
                <span className="text-section tabular-nums">
                  {activeCount}
                  <span className="font-normal text-fg-3">
                    {" "}
                    / {supported.length}
                  </span>
                </span>{" "}
                {t("aicli.clients.summary")}
              </span>
            </div>
            <span className="text-caption text-fg-3">
              {t("aicli.clients.restoreHint")}
            </span>
          </div>
          {rest.length > 0 && (
            <BulkTakeover
              apps={rest}
              disabled={!bound}
              onDone={setBulkResult}
            />
          )}
        </div>

        {proxyDown && (
          <Notice tone="danger" title={t("aicli.clients.proxyDown")} />
        )}
        {bulkResult && bulkResult.conflicts.length > 0 && (
          <Notice
            tone="warning"
            title={t("aicli.clients.bulkConflict", {
              apps: names(bulkResult.conflicts),
            })}
          />
        )}
        {bulkResult && bulkResult.failed.length > 0 && (
          <Notice
            tone="danger"
            title={t("aicli.clients.bulkFailed", {
              apps: names(bulkResult.failed),
            })}
          />
        )}

        <section
          aria-labelledby="aicli-clients-supported"
          className="flex flex-col gap-3"
        >
          <h2
            id="aicli-clients-supported"
            className="m-0 text-caption font-semibold text-fg-2"
          >
            {t("aicli.clients.supportedTitle")}
          </h2>
          <div className="overflow-x-auto rounded-panel border border-border bg-surface">
            <div className="min-w-[640px]">
              <div
                aria-hidden="true"
                className={cn(
                  ROW_GRID,
                  "border-b border-border px-4 py-2 text-badge font-normal text-fg-2",
                )}
              >
                <span>{t("aicli.clients.colClient")}</span>
                <span>{t("aicli.clients.colRoute")}</span>
                <span>{t("aicli.clients.colRestore")}</span>
                <span className="text-end">{t("aicli.clients.colSwitch")}</span>
              </div>
              {supported.map((app) => {
                const takenAt = entryOf(app)?.taken_over_at ?? null;
                return (
                  <ClientRow
                    key={app}
                    app={app}
                    bound={bound}
                    lineId={account?.selected_line_id ?? "main"}
                    takenAt={
                      takenAt
                        ? new Date(takenAt * 1000).toLocaleString(
                            i18n.language,
                            {
                              month: "short",
                              day: "numeric",
                              hour: "2-digit",
                              minute: "2-digit",
                            },
                          )
                        : null
                    }
                    onOpen={() => onOpenApp(app)}
                  />
                );
              })}
            </div>
          </div>
        </section>

        {dedicated.length > 0 && (
          <section
            aria-labelledby="aicli-clients-other"
            className="flex flex-col gap-3"
          >
            <h2
              id="aicli-clients-other"
              className="m-0 text-caption font-semibold text-fg-2"
            >
              {t("aicli.clients.otherTitle")}
            </h2>
            <div className="grid grid-cols-[repeat(auto-fill,minmax(240px,1fr))] gap-3">
              {dedicated.map((app) => (
                <button
                  key={app}
                  type="button"
                  data-testid={`aicli-client-other-${app}`}
                  onClick={() => onOpenApp(app)}
                  className="group flex items-center gap-3 rounded-panel border border-border bg-surface px-4 py-3 text-start transition-colors hover:border-border-strong"
                >
                  <AppGlyph app={app} size={22} badgeClassName="bg-surface" />
                  <span className="flex min-w-0 flex-1 flex-col">
                    <span className="truncate text-strong text-fg-1">
                      {APP_DISPLAY_NAME[app]}
                    </span>
                    <span className="truncate text-caption text-fg-3">
                      {t(`aicli.clients.dedicated.${app}`)}
                    </span>
                  </span>
                  <ArrowRight
                    aria-hidden="true"
                    className="h-4 w-4 shrink-0 text-fg-3 transition-transform group-hover:translate-x-0.5"
                  />
                </button>
              ))}
            </div>
          </section>
        )}

        {comingSoon.length > 0 && (
          <section
            aria-labelledby="aicli-clients-soon"
            className="flex flex-col gap-2"
          >
            <h2
              id="aicli-clients-soon"
              className="m-0 text-caption font-semibold text-fg-2"
            >
              {t("aicli.clients.comingSoonTitle")}
            </h2>
            <ul
              data-testid="aicli-clients-coming-soon"
              className="m-0 flex list-none flex-wrap gap-2 p-0"
            >
              {comingSoon.map((app) => (
                <li
                  key={app}
                  className="inline-flex h-7 items-center gap-1.5 rounded-full border border-dashed border-border px-2.5 text-caption text-fg-3"
                >
                  <AppGlyph
                    app={app}
                    size={14}
                    badgeClassName="bg-app"
                    className="opacity-70 grayscale"
                  />
                  {APP_DISPLAY_NAME[app]}
                </li>
              ))}
            </ul>
            <p className="m-0 text-caption text-fg-3">
              {t("aicli.clients.comingSoonHint")}
            </p>
          </section>
        )}
      </div>
    </div>
  );
}

/**
 * 一次接管其余的客户端：确认对话框列出会改写哪几个，确认后逐个接管。
 * 遇到旧代理配置冲突的不强行覆盖，留给用户在那一行单独确认；其余失败的一并报告。
 */
function BulkTakeover({
  apps,
  disabled,
  onDone,
}: {
  apps: AppId[];
  disabled: boolean;
  onDone: (result: BulkResult) => void;
}) {
  const { t } = useTranslation();
  const takeOver = useAicliTakeOver();
  const [open, setOpen] = useState(false);
  const [running, setRunning] = useState(false);
  const account = useAicliAccount().data;
  const profiles = useAicliProfiles(open);
  const routes = useAicliRoutes(open);
  const profileNameOf = (app: AppId) => {
    const { id, profile } = resolveClientProfile(
      app,
      account,
      profiles.data,
      routes.data,
    );
    return aicliProfileName(t, profile, id);
  };

  const run = async () => {
    setRunning(true);
    const result: BulkResult = { conflicts: [], failed: [] };
    for (const app of apps) {
      try {
        await takeOver.mutateAsync({ app, confirmed: false });
      } catch (failure) {
        if (aicliErrorCode(failure) === "cc_switch_conflict") {
          result.conflicts.push(app);
        } else {
          result.failed.push(app);
        }
      }
    }
    setRunning(false);
    setOpen(false);
    onDone(result);
  };

  return (
    <>
      <Button
        type="button"
        variant="neutral"
        size="regular"
        data-testid="aicli-clients-takeover-rest"
        disabled={disabled || running}
        onClick={() => setOpen(true)}
      >
        {t("aicli.clients.takeOverRest", { count: apps.length })}
      </Button>
      <Dialog
        open={open}
        onOpenChange={(next) => {
          if (!next && !running) setOpen(false);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {t("aicli.clients.bulkTitle", { count: apps.length })}
            </DialogTitle>
            <DialogDescription>{t("aicli.clients.bulkBody")}</DialogDescription>
          </DialogHeader>
          {/* 正文（2026-11 改版，设计稿画板 22）：一行一个客户端和它将用的配置集 */}
          <div className="flex flex-col gap-3 px-6 pb-2">
            <ul
              data-testid="aicli-bulk-apps"
              className="m-0 flex list-none flex-col overflow-hidden rounded-control border border-border p-0"
            >
              {apps.map((app) => (
                <li
                  key={app}
                  className="flex items-center gap-2.5 border-b border-border px-3.5 py-2.5 last:border-b-0"
                >
                  <AppGlyph app={app} size={18} badgeClassName="bg-surface" />
                  <span className="min-w-0 flex-1 truncate text-strong text-fg-1">
                    {APP_DISPLAY_NAME[app]}
                  </span>
                  <span className="shrink-0 truncate text-caption text-fg-3">
                    {t("aicli.clients.bulkProfile", {
                      name: profileNameOf(app),
                    })}
                  </span>
                </li>
              ))}
            </ul>
            <p className="m-0 flex items-start gap-2 text-caption text-fg-3">
              <Info
                aria-hidden="true"
                className="mt-0.5 h-3.5 w-3.5 shrink-0"
              />
              <span>{t("aicli.clients.bulkSkipNote")}</span>
            </p>
          </div>
          <DialogFooter>
            <Button
              type="button"
              variant="neutral"
              size="regular"
              disabled={running}
              onClick={() => setOpen(false)}
            >
              {t("aicli.client.cancel")}
            </Button>
            <Button
              type="button"
              variant="solid"
              size="regular"
              disabled={running}
              onClick={() => void run()}
            >
              {running && (
                <Loader2
                  aria-hidden="true"
                  className="h-3.5 w-3.5 animate-spin"
                />
              )}
              {t("aicli.client.takeOver")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}

function ClientRow({
  app,
  bound,
  lineId,
  takenAt,
  onOpen,
}: {
  app: AppId;
  bound: boolean;
  /** 当前线路 id：已接管的客户端经它发往 AICLI。 */
  lineId: string;
  /** 接管时间（已格式化）；接管前的配置就是那时备份的。 */
  takenAt: string | null;
  onOpen: () => void;
}) {
  const { t } = useTranslation();
  const account = useAicliAccount().data;
  const profiles = useAicliProfiles(bound);
  const routes = useAicliRoutes(bound);
  const flow = useAicliTakeoverFlow(app, bound);
  const name = APP_DISPLAY_NAME[app];
  const { id, profile } = resolveClientProfile(
    app,
    account,
    profiles.data,
    routes.data,
  );
  const profileName = aicliProfileName(t, profile, id);
  const models = profile?.routes.length ?? 0;
  const state = flow.conflict ? "conflict" : flow.display;
  const active = flow.display === "active";

  return (
    <article
      data-testid={`aicli-client-card-${app}`}
      data-state={state}
      className={cn(
        ROW_GRID,
        "relative border-b border-border px-4 py-3 transition-colors last:border-b-0 hover:bg-subtle",
        state === "active" && "bg-action-soft hover:bg-action-soft",
        state === "conflict" && "bg-warning-soft hover:bg-warning-soft",
      )}
    >
      {/* 整行可点进详情；开关和按钮叠在上面，避免按钮套按钮 */}
      <button
        type="button"
        aria-label={t("aicli.clients.open", { app: name })}
        onClick={onOpen}
        className="absolute inset-0 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
      />
      <div className="pointer-events-none flex min-w-0 items-center gap-3">
        <AppGlyph
          app={app}
          size={24}
          badgeClassName={active ? "bg-action-soft" : "bg-surface"}
        />
        <h3 className="m-0 truncate text-strong text-fg-1">{name}</h3>
      </div>
      <div className="pointer-events-none flex min-w-0 flex-col gap-0.5">
        <span
          data-testid={`aicli-client-state-${app}`}
          className={cn(
            "inline-flex items-center gap-1.5 text-body font-medium",
            state === "active"
              ? "text-action-text"
              : state === "conflict"
                ? "text-warning-text"
                : "text-fg-1",
          )}
        >
          {state === "pending" && (
            <Loader2 aria-hidden="true" className="h-3.5 w-3.5 animate-spin" />
          )}
          {t(`aicli.clients.status.${state}`)}
        </span>
        <span className="truncate text-caption text-fg-2">
          {state === "conflict"
            ? t("aicli.client.conflictNotice", { app: name })
            : active
              ? `${t("aicli.lines.aicliSource", {
                  line: t(`aicli.lines.${lineId}`, { defaultValue: lineId }),
                })} · ${t("aicli.clients.profileLine", {
                  name: profileName,
                  count: models,
                })}`
              : t("aicli.clients.afterProfileLine", {
                  name: profileName,
                  count: models,
                })}
        </span>
        {flow.error && (
          <span role="alert" className="text-caption text-danger-text">
            {flow.error}
          </span>
        )}
      </div>
      <div className="pointer-events-none flex min-w-0 flex-col gap-0.5">
        {active ? (
          <>
            <span className="truncate text-body text-fg-1">
              {t("aicli.clients.restoreTo")}
            </span>
            {takenAt && (
              <span className="truncate text-caption text-fg-3">
                {t("aicli.clients.restoreBackedUp", { time: takenAt })}
              </span>
            )}
          </>
        ) : (
          <span aria-hidden="true" className="text-body text-fg-3">
            —
          </span>
        )}
      </div>
      <div className="relative flex justify-end">
        {state === "conflict" ? (
          <Button
            type="button"
            variant="neutral"
            size="compact"
            disabled={!bound || flow.busy}
            onClick={() => flow.request("conflict")}
          >
            {t("aicli.client.overrideAndTakeOver")}
          </Button>
        ) : (
          <Switch
            checked={active}
            aria-label={t("aicli.clients.switchLabel", { app: name })}
            disabled={!bound || flow.busy || flow.display === "pending"}
            onCheckedChange={(next) =>
              flow.request(next ? "takeover" : "release")
            }
          />
        )}
      </div>
      <AicliTakeoverDialog flow={flow} />
    </article>
  );
}
