import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { Loader2, RefreshCw } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Notice } from "@/components/ui/notice";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { APP_DISPLAY_NAME } from "@/components/shell/AppGlyph";
import type { AppId } from "@/lib/api";
import { aicliApi, aicliErrorCode, isAicliCommandError } from "@/lib/api/aicli";
import { aicliKeys } from "@/hooks/aicliKeys";
import { useAicliAccount } from "@/hooks/useAicliAccount";
import {
  useAicliCatalog,
  useAicliProfiles,
  useAicliRelease,
  useAicliRoutes,
  useAicliTakeOver,
  useAicliTakeoverStatus,
} from "@/hooks/useAicliProfiles";
import type {
  AicliCatalog,
  AicliCompatibility,
  AicliModelRoute,
} from "@/types/aicli";

/** 支持 AICLI 接管的客户端（后端 mode::controller::PROXY_APPS）。 */
export const AICLI_TAKEOVER_APPS: readonly AppId[] = [
  "claude",
  "codex",
  "gemini",
  "grokbuild",
  "claude-desktop",
];

/** 客户端走的线协议（后端 takeover 投影同一映射），决定看哪一列兼容级别。 */
export function protocolForApp(app: AppId): string {
  if (app === "claude" || app === "claude-desktop") return "anthropic";
  if (app === "gemini") return "gemini";
  return "openai-response";
}

export type AicliTakeoverDisplay = "active" | "pending" | "inactive";

export function takeoverDisplay(
  state: string | undefined,
): AicliTakeoverDisplay {
  if (state === "active") return "active";
  if (state === "taking_over" || state === "releasing") return "pending";
  return "inactive";
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

type PendingDialog = "takeover" | "conflict" | "release" | null;

/**
 * 客户端页（F-04，取代供应商列表）：接管状态（已接管 / 未接管 / 冲突）、接管与
 * 停止接管（确认对话框说明会还原配置）、当前配置集、本客户端可用模型表与「刷新
 * 模型目录」。
 */
export function AicliClientPage({ app }: { app: AppId }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const appName = APP_DISPLAY_NAME[app] ?? app;
  const supported = AICLI_TAKEOVER_APPS.includes(app);

  const account = useAicliAccount().data;
  const bound = account?.state === "bound" || account?.state === "needs_rebind";
  const takeover = useAicliTakeoverStatus(bound && supported);
  const profiles = useAicliProfiles(bound);
  const routes = useAicliRoutes(bound);
  const catalog = useAicliCatalog(bound);
  const takeOver = useAicliTakeOver();
  const release = useAicliRelease();

  const [dialog, setDialog] = useState<PendingDialog>(null);
  const [conflict, setConflict] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  const state = takeover.data?.find((entry) => entry.app === app)?.state;
  const display = takeoverDisplay(state);

  // 本客户端的配置集：客户端专用绑定 > 账户默认 > 推荐预设
  const appBinding = profiles.data?.bindings.find(
    (binding) => binding.scope === "app" && binding.scope_key === app,
  );
  const profileId =
    appBinding?.profile_id ??
    account?.active_profile_id ??
    routes.data?.active_profile_id ??
    "recommended";
  const profile = routes.data?.profiles.find((entry) => entry.id === profileId);
  const profileName =
    profile?.name ??
    (profileId === "recommended" ? t("aicli.profile.recommended") : profileId);
  const lineId = routes.data?.selected_line_id ?? account?.selected_line_id;
  const lineName = lineId ? t(`aicli.lines.${lineId}`) : "";
  const protocol = protocolForApp(app);
  const models = profile?.routes ?? [];
  const partialCount = models.filter(
    (route) => compatOf(catalog.data, route, protocol) === "partial",
  ).length;

  const failMessage = (failure: unknown) =>
    t("aicli.client.failed", {
      message: isAicliCommandError(failure) ? failure.message : String(failure),
    });

  const runTakeOver = (confirmed: boolean) => {
    setError(null);
    takeOver.mutate(
      { app, confirmed },
      {
        onSuccess: () => {
          setConflict(false);
          setDialog(null);
        },
        onError: (failure) => {
          if (!confirmed && aicliErrorCode(failure) === "cc_switch_conflict") {
            setConflict(true);
            setDialog("conflict");
            return;
          }
          setDialog(null);
          setError(failMessage(failure));
        },
      },
    );
  };

  const runRelease = () => {
    setError(null);
    release.mutate(app, {
      onSettled: () => setDialog(null),
      onError: (failure) => setError(failMessage(failure)),
    });
  };

  const refreshCatalog = async () => {
    setRefreshing(true);
    setError(null);
    try {
      await aicliApi.refresh("all");
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: aicliKeys.catalog }),
        queryClient.invalidateQueries({ queryKey: aicliKeys.routes }),
      ]);
    } catch (failure) {
      setError(failMessage(failure));
    } finally {
      setRefreshing(false);
    }
  };

  const busy = takeOver.isPending || release.isPending;
  const statusLabel = conflict
    ? t("aicli.client.status.conflict")
    : t(`aicli.client.status.${display}`);

  return (
    <div
      id="main-content"
      data-testid="aicli-client-page"
      className="min-h-0 flex-1 overflow-y-auto scroll-stable px-6 pb-12 pt-4"
    >
      <div className="flex flex-col gap-4">
        {!bound && account && (
          <Notice tone="neutral" title={t("aicli.client.notBound")} />
        )}

        <section
          aria-labelledby="aicli-client-takeover"
          className="flex flex-col gap-3 rounded-panel border border-border bg-surface p-4"
        >
          <div className="flex flex-wrap items-center gap-2">
            <h2
              id="aicli-client-takeover"
              className="m-0 text-section text-fg-1"
            >
              {t("aicli.client.takeoverTitle")}
            </h2>
            {supported && bound && (
              <Badge
                variant="outline"
                data-testid="aicli-takeover-state"
                className="h-5 px-1.5 text-badge"
              >
                {statusLabel}
              </Badge>
            )}
            {supported && bound && (
              <div className="ms-auto flex items-center gap-2">
                {display === "active" ? (
                  <Button
                    type="button"
                    variant="neutral"
                    size="compact"
                    disabled={busy}
                    onClick={() => setDialog("release")}
                  >
                    {t("aicli.client.release")}
                  </Button>
                ) : (
                  <Button
                    type="button"
                    variant="solid"
                    size="compact"
                    disabled={busy || display === "pending"}
                    onClick={() => setDialog("takeover")}
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
          <p className="m-0 text-body text-fg-2">
            {t("aicli.client.currentProfile", { name: profileName })}
          </p>
        </section>

        {bound && (
          <section
            aria-labelledby="aicli-client-models"
            className="flex flex-col gap-3 rounded-panel border border-border bg-surface p-4"
          >
            <div className="flex items-center gap-2">
              <h2
                id="aicli-client-models"
                className="m-0 text-section text-fg-1"
              >
                {t("aicli.client.modelsTitle")}
              </h2>
              <Button
                type="button"
                variant="quiet"
                size="compact"
                className="ms-auto"
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
            {app === "claude" && partialCount > 0 && (
              <Notice
                tone="warning"
                title={t("aicli.client.partialNotice", { count: partialCount })}
              />
            )}
            {models.length === 0 ? (
              <p className="m-0 text-body text-fg-2">
                {t("aicli.client.noModels")}
              </p>
            ) : (
              <table className="w-full border-collapse text-body">
                <thead>
                  <tr className="text-start text-caption text-fg-3">
                    <th className="py-1 pe-3 text-start font-normal">
                      {t("aicli.client.columns.model")}
                    </th>
                    <th className="py-1 pe-3 text-start font-normal">
                      {t("aicli.client.columns.profile")}
                    </th>
                    <th className="py-1 pe-3 text-start font-normal">
                      {t("aicli.client.columns.line")}
                    </th>
                    <th className="py-1 pe-3 text-start font-normal">
                      {t("aicli.client.columns.compat")}
                    </th>
                    <th className="py-1 text-start font-normal">
                      {t("aicli.client.columns.capabilities")}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {models.map((route) => {
                    const compat = compatOf(catalog.data, route, protocol);
                    const tags = capabilityTags(catalog.data, route);
                    return (
                      <tr
                        key={route.public_model}
                        data-testid={`aicli-model-${route.public_model}`}
                        className="border-t border-border"
                      >
                        <td className="py-1.5 pe-3 text-fg-1">
                          {route.public_model}
                        </td>
                        <td className="py-1.5 pe-3 text-fg-2">{profileName}</td>
                        <td className="py-1.5 pe-3 text-fg-2">{lineName}</td>
                        <td className="py-1.5 pe-3 text-fg-2">
                          {t(`aicli.client.compat.${compat}`)}
                        </td>
                        <td className="py-1.5 text-fg-2">
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
            )}
          </section>
        )}
      </div>

      <Dialog
        open={dialog !== null}
        onOpenChange={(open) => {
          if (!open && !busy) setDialog(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {dialog === "release"
                ? t("aicli.client.confirmReleaseTitle", { app: appName })
                : dialog === "conflict"
                  ? t("aicli.client.conflictTitle")
                  : t("aicli.client.confirmTakeOverTitle", { app: appName })}
            </DialogTitle>
            <DialogDescription>
              {dialog === "release"
                ? t("aicli.client.confirmReleaseBody", { app: appName })
                : dialog === "conflict"
                  ? t("aicli.client.conflictBody", { app: appName })
                  : t("aicli.client.confirmTakeOverBody", { app: appName })}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              type="button"
              variant="neutral"
              size="regular"
              disabled={busy}
              onClick={() => setDialog(null)}
            >
              {t("aicli.client.cancel")}
            </Button>
            <Button
              type="button"
              variant="solid"
              size="regular"
              disabled={busy}
              onClick={() =>
                dialog === "release"
                  ? runRelease()
                  : runTakeOver(dialog === "conflict")
              }
            >
              {busy && (
                <Loader2
                  aria-hidden="true"
                  className="h-3.5 w-3.5 animate-spin"
                />
              )}
              {dialog === "release"
                ? t("aicli.client.release")
                : dialog === "conflict"
                  ? t("aicli.client.overrideAndTakeOver")
                  : t("aicli.client.takeOver")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
