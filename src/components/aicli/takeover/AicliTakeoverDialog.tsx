import { useTranslation } from "react-i18next";
import { Check, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { APP_DISPLAY_NAME, AppGlyph } from "@/components/shell/AppGlyph";
import { cn } from "@/lib/utils";
import { useAicliAccount } from "@/hooks/useAicliAccount";
import {
  useAicliProfiles,
  useAicliRoutes,
  useAicliTakeoverStatus,
} from "@/hooks/useAicliProfiles";
import { aicliProfileName } from "../profiles/profileName";
import { resolveClientProfile } from "./clientProfile";
import type { AicliTakeoverFlow } from "./useAicliTakeoverFlow";

/**
 * 接管 / 停止接管 / 冲突覆盖的确认对话框（2026-11 改版，设计稿画板 19）。
 * - 接管：三条承诺（请求改走本机、先完整备份、随时原样还原）和将使用的配置集。
 * - 停止接管：会回到接管前的配置，写明备份时间；之后请求不再经过 AICLI。
 * - 冲突：旧版本地代理留下的设置会被覆盖，也一并备份、停止接管时还原。
 */
export function AicliTakeoverDialog({ flow }: { flow: AicliTakeoverFlow }) {
  const { t, i18n } = useTranslation();
  const appName = APP_DISPLAY_NAME[flow.app] ?? flow.app;
  const { dialog, busy } = flow;
  const open = dialog !== null;

  const account = useAicliAccount().data;
  const profiles = useAicliProfiles(open && dialog === "takeover");
  const routes = useAicliRoutes(open && dialog === "takeover");
  const status = useAicliTakeoverStatus(open && dialog === "release");

  const { id: profileId, profile } = resolveClientProfile(
    flow.app,
    account,
    profiles.data,
    routes.data,
  );
  const profileName = aicliProfileName(
    t,
    profiles.data?.profiles.find((item) => item.id === profileId) ??
      profiles.data?.presets.find((item) => item.id === profileId) ??
      profile,
    profileId,
  );
  const ownProfile = profiles.data?.bindings.some(
    (binding) => binding.scope === "app" && binding.scope_key === flow.app,
  );
  const takenAt = status.data?.find(
    (entry) => entry.app === flow.app,
  )?.taken_over_at;

  const title =
    dialog === "release"
      ? t("aicli.client.confirmReleaseTitle", { app: appName })
      : dialog === "conflict"
        ? t("aicli.client.conflictTitle", { app: appName })
        : t("aicli.client.confirmTakeOverTitle", { app: appName });

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) flow.cancel();
      }}
    >
      <DialogContent>
        <DialogHeader>
          <div className="flex items-center gap-3">
            <AppGlyph app={flow.app} size={32} badgeClassName="bg-surface" />
            <DialogTitle>{title}</DialogTitle>
          </div>
        </DialogHeader>

        {dialog === "takeover" && (
          <div className="flex flex-col gap-3 px-6 pb-2">
            <DialogDescription asChild>
              <ul className="m-0 flex list-none flex-col gap-2 p-0 text-body text-fg-1">
                {(
                  [
                    "takeOverPoint1",
                    "takeOverPoint2",
                    "takeOverPoint3",
                  ] as const
                ).map((key) => (
                  <li key={key} className="flex gap-2">
                    <Check
                      aria-hidden="true"
                      className="mt-0.5 h-4 w-4 shrink-0 text-action-text"
                      strokeWidth={2.4}
                    />
                    <span>{t(`aicli.client.${key}`)}</span>
                  </li>
                ))}
              </ul>
            </DialogDescription>
            <p className="m-0 text-caption text-fg-3">
              {ownProfile
                ? t("aicli.client.takeOverProfile", { name: profileName })
                : t("aicli.client.takeOverProfileDefault", {
                    name: profileName,
                  })}
            </p>
          </div>
        )}

        {dialog === "release" && (
          <div className="flex flex-col gap-3 px-6 pb-2">
            <DialogDescription className="text-body text-fg-1">
              {t("aicli.client.releaseLead")}
            </DialogDescription>
            <div className="flex flex-col gap-0.5 rounded-panel bg-subtle px-4 py-3">
              <span className="text-strong font-semibold text-fg-1">
                {t("aicli.clients.restoreTo")}
              </span>
              {takenAt != null && (
                <span className="text-caption text-fg-2">
                  {t("aicli.clients.restoreBackedUp", {
                    time: new Date(takenAt * 1000).toLocaleString(
                      i18n.language,
                    ),
                  })}
                </span>
              )}
            </div>
            <p className="m-0 text-caption text-fg-3">
              {t("aicli.client.releaseAfter")}
            </p>
          </div>
        )}

        {dialog === "conflict" && (
          <div className="flex flex-col gap-3 px-6 pb-2">
            <div className="flex flex-col gap-1 rounded-panel bg-warning-soft px-4 py-3">
              <span className="text-strong font-semibold text-warning-text">
                {t("aicli.client.conflictBoxTitle")}
              </span>
              <span className="text-caption text-fg-2">
                {t("aicli.client.conflictBoxBody")}
              </span>
            </div>
            <DialogDescription className="text-body text-fg-1">
              {t("aicli.client.conflictBody", { app: appName })}
            </DialogDescription>
          </div>
        )}

        <DialogFooter>
          <Button
            type="button"
            variant="neutral"
            size="regular"
            disabled={busy}
            onClick={flow.cancel}
          >
            {t("aicli.client.cancel")}
          </Button>
          <Button
            type="button"
            // 接管是主操作（实心）；停止接管与覆盖都是要想一下的操作，用描边
            variant={dialog === "takeover" ? "solid" : "neutral"}
            size="regular"
            className={cn(
              dialog === "release" && "border-fg-1 font-semibold",
              dialog === "conflict" &&
                "border-warning font-semibold text-warning-text hover:text-warning-text",
            )}
            disabled={busy}
            onClick={flow.confirm}
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
  );
}
