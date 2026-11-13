import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { getVersion } from "@tauri-apps/api/app";
import { Check, Sparkles } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { useSettingsQuery } from "@/lib/query";
import { settingsApi } from "@/lib/api";
import { markSeen } from "@/lib/whatsNew";

/** 首次运行欢迎提示：仅当后端启动阶段保留 firstRunNoticeConfirmed 为空时弹出。 */
export function FirstRunNoticeDialog() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const { data: settings } = useSettingsQuery();

  // 后端启动时已经决定好要不要弹：条件不满足的话字段会立即被写成 true，
  // 所以前端这里只需要判空即可——与其他既有确认标记的模式一致。
  const isOpen = settings != null && settings.firstRunNoticeConfirmed !== true;

  const handleAcknowledge = async () => {
    if (!settings) return;
    try {
      const rest = { ...settings };
      // 新装用户看的就是新界面、新版本，「界面改版了」弹窗和更新摘要一并记成已看过
      const version = await getVersion().catch(() => undefined);
      await settingsApi.save({
        ...rest,
        firstRunNoticeConfirmed: true,
        newLayoutNoticeConfirmed: true,
        whatsNewSeenVersion: markSeen(settings.whatsNewSeenVersion, version),
      });
      await queryClient.invalidateQueries({ queryKey: ["settings"] });
    } catch (error) {
      console.error("Failed to save firstRunNoticeConfirmed:", error);
    }
  };

  return (
    <Dialog
      open={isOpen}
      onOpenChange={(open) => {
        if (!open) void handleAcknowledge();
      }}
    >
      <DialogContent className="max-w-md" zIndex="top">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Sparkles className="h-5 w-5 text-fg-1" />
            {t("firstRunNotice.title")}
          </DialogTitle>
        </DialogHeader>
        {/* 正文（2026-11 改版，设计稿画板 21）：账户已接上，接下来按需做的三步 */}
        <div className="flex flex-col gap-4 px-6 py-4">
          <DialogDescription className="text-sm text-fg-2">
            {t("firstRunNotice.bodyDefault")}
          </DialogDescription>
          <ol className="m-0 flex list-none flex-col gap-3.5 p-0">
            <li className="flex gap-3">
              <span
                aria-hidden="true"
                className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-action text-action-fg"
              >
                <Check className="h-3.5 w-3.5" strokeWidth={3} />
              </span>
              <span className="flex flex-col">
                <span className="text-strong text-fg-1">
                  {t("firstRunNotice.stepAccount")}
                </span>
                <span className="text-caption text-action-text">
                  {t("firstRunNotice.stepDone")}
                </span>
              </span>
            </li>
            <li className="flex gap-3">
              <span
                aria-hidden="true"
                className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full border-[1.5px] border-action text-caption font-semibold text-action-text"
              >
                2
              </span>
              <span className="flex flex-col">
                <span className="text-strong text-fg-1">
                  {t("firstRunNotice.stepTakeover")}
                </span>
                <span className="text-caption leading-relaxed text-fg-2">
                  {t("firstRunNotice.bodyOfficial")}
                </span>
              </span>
            </li>
            <li className="flex gap-3">
              <span
                aria-hidden="true"
                className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full border-[1.5px] border-border-strong text-caption font-semibold text-fg-2"
              >
                3
              </span>
              <span className="flex flex-col">
                <span className="text-strong text-fg-1">
                  {t("firstRunNotice.stepPrivacy")}
                </span>
                <span className="text-caption leading-relaxed text-fg-2">
                  {t("firstRunNotice.stepPrivacyBody")}
                </span>
              </span>
            </li>
          </ol>
        </div>
        <DialogFooter>
          <Button onClick={handleAcknowledge}>
            {t("firstRunNotice.confirm")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
