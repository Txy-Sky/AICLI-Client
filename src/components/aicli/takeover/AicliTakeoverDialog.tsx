import { useTranslation } from "react-i18next";
import { Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { APP_DISPLAY_NAME } from "@/components/shell/AppGlyph";
import type { AicliTakeoverFlow } from "./useAicliTakeoverFlow";

/**
 * 接管 / 停止接管 / 冲突覆盖的确认对话框。停止接管的说明写明会还原为接管前的配置
 * （例如官方登录），用户随时可以回到原来的用法。
 */
export function AicliTakeoverDialog({ flow }: { flow: AicliTakeoverFlow }) {
  const { t } = useTranslation();
  const appName = APP_DISPLAY_NAME[flow.app] ?? flow.app;
  const { dialog, busy } = flow;

  return (
    <Dialog
      open={dialog !== null}
      onOpenChange={(open) => {
        if (!open) flow.cancel();
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
            onClick={flow.cancel}
          >
            {t("aicli.client.cancel")}
          </Button>
          <Button
            type="button"
            variant="solid"
            size="regular"
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
