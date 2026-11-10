import { useTranslation } from "react-i18next";
import { ArrowRight, Download, Loader2, MonitorSmartphone } from "lucide-react";
import { AppPageHeader } from "@/components/shell/AppPageHeader";
import { APP_DISPLAY_NAME, AppGlyph } from "@/components/shell/AppGlyph";
import { Button } from "@/components/ui/button";
import { HelpTip } from "@/components/ui/help-tip";
import { Notice } from "@/components/ui/notice";
import { orderAppIds } from "@/config/appConfig";
import type { AppId } from "@/lib/api";
import { cn } from "@/lib/utils";
import { useProxyStatusQuery } from "@/lib/query/proxy";
import { useAicliAccount } from "@/hooks/useAicliAccount";
import {
  useAicliProfiles,
  useAicliRoutes,
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

/** 有自己专属页（工作区、配置、记忆）的客户端：还没有 AICLI 接管，但页面仍可进入。 */
const DEDICATED_PAGE_APPS: readonly AppId[] = ["openclaw", "hermes"];

interface AicliClientsPageProps {
  visibleApps: VisibleApps;
  appOrder?: string[];
  onOpenApp: (app: AppId) => void;
  onOpenApps: () => void;
}

/**
 * 客户端总览（F-08）：侧栏只剩一个「客户端」入口，进来是卡片网格。支持接管的客户端
 * 一张卡：状态、所用配置集与模型数、一键接管 / 停止接管（停止接管会还原为接管前的
 * 配置，例如官方登录）；点卡片进入详情页。有专属页的客户端单列一行，其余客户端列为
 * 「即将支持」。
 */
export function AicliClientsPage({
  visibleApps,
  appOrder,
  onOpenApp,
  onOpenApps,
}: AicliClientsPageProps) {
  const { t } = useTranslation();
  const account = useAicliAccount().data;
  const bound = account?.state === "bound" || account?.state === "needs_rebind";
  const takeover = useAicliTakeoverStatus(bound);
  const { data: proxyStatus } = useProxyStatusQuery();

  const ordered = orderAppIds(appOrder).filter((app) => visibleApps[app]);
  const supported = ordered.filter((app) => AICLI_TAKEOVER_APPS.includes(app));
  const dedicated = ordered.filter((app) => DEDICATED_PAGE_APPS.includes(app));
  const comingSoon = ordered.filter(
    (app) =>
      !AICLI_TAKEOVER_APPS.includes(app) && !DEDICATED_PAGE_APPS.includes(app),
  );

  const activeCount = supported.filter(
    (app) =>
      takeoverDisplay(
        takeover.data?.find((entry) => entry.app === app)?.state,
      ) === "active",
  ).length;
  const proxyDown =
    activeCount > 0 && proxyStatus !== undefined && !proxyStatus.running;

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
        <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
          <span className="text-metric tabular-nums text-fg-1">
            {activeCount}
            <span className="text-section font-normal text-fg-3">
              {" "}
              / {supported.length}
            </span>
          </span>
          <span className="text-body text-fg-2">
            {t("aicli.clients.summary")}
          </span>
          <span className="basis-full text-caption text-fg-3">
            {t("aicli.clients.restoreHint")}
          </span>
        </div>

        {proxyDown && (
          <Notice tone="danger" title={t("aicli.clients.proxyDown")} />
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
          <div className="grid grid-cols-[repeat(auto-fill,minmax(240px,1fr))] gap-3">
            {supported.map((app) => (
              <ClientCard
                key={app}
                app={app}
                bound={bound}
                onOpen={() => onOpenApp(app)}
              />
            ))}
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

function ClientCard({
  app,
  bound,
  onOpen,
}: {
  app: AppId;
  bound: boolean;
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
  const models = profile?.routes.length ?? 0;
  const state = flow.conflict ? "conflict" : flow.display;

  const pill = {
    active: "bg-success-soft text-success-text",
    inactive: "bg-subtle text-fg-3",
    pending: "bg-subtle text-fg-2",
    conflict: "bg-warning-soft text-warning-text",
  }[state];
  const dot = {
    active: "bg-success",
    inactive: "bg-fg-3",
    pending: "bg-fg-2",
    conflict: "bg-warning",
  }[state];

  return (
    <article
      data-testid={`aicli-client-card-${app}`}
      data-state={state}
      className={cn(
        "relative flex min-w-0 flex-col gap-3 rounded-panel border bg-surface p-4 transition-colors hover:border-border-strong",
        state === "active" ? "border-border-strong" : "border-border",
      )}
    >
      {/* 整张卡可点进详情；按钮叠在上面，避免按钮套按钮 */}
      <button
        type="button"
        aria-label={t("aicli.clients.open", { app: name })}
        onClick={onOpen}
        className="absolute inset-0 rounded-panel focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      />
      <div className="pointer-events-none flex items-center gap-3">
        <AppGlyph app={app} size={28} badgeClassName="bg-surface" />
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <h3 className="m-0 truncate text-strong text-fg-1">{name}</h3>
          <span
            data-testid={`aicli-client-state-${app}`}
            className={cn(
              "inline-flex h-5 w-fit items-center gap-1.5 rounded-full px-2 text-badge font-medium",
              pill,
            )}
          >
            {state === "pending" ? (
              <Loader2 aria-hidden="true" className="h-3 w-3 animate-spin" />
            ) : (
              <span
                aria-hidden="true"
                className={cn("h-1.5 w-1.5 rounded-full", dot)}
              />
            )}
            {t(`aicli.clients.status.${state}`)}
          </span>
        </div>
      </div>
      <p className="pointer-events-none m-0 truncate text-caption text-fg-2">
        {t("aicli.clients.profileLine", {
          name: aicliProfileName(t, profile, id),
          count: models,
        })}
      </p>
      {flow.error && (
        <p
          role="alert"
          className="pointer-events-none relative m-0 text-caption text-danger-text"
        >
          {flow.error}
        </p>
      )}
      <div className="relative flex">
        {flow.display === "active" ? (
          <Button
            type="button"
            variant="neutral"
            size="compact"
            disabled={!bound || flow.busy}
            onClick={() => flow.request("release")}
          >
            {t("aicli.client.release")}
          </Button>
        ) : (
          <Button
            type="button"
            variant="solid"
            size="compact"
            disabled={!bound || flow.busy || flow.display === "pending"}
            onClick={() => flow.request("takeover")}
          >
            {t("aicli.client.takeOver")}
          </Button>
        )}
      </div>
      <AicliTakeoverDialog flow={flow} />
    </article>
  );
}
