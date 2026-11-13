import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, Loader2, RotateCcw, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  SettingsBlock,
  SettingsCard,
} from "@/components/settings/SettingsLayout";
import { cn } from "@/lib/utils";
import { formatAicliQuota, readAccountQuota } from "@/lib/aicliQuota";
import { isAicliCommandError } from "@/lib/api/aicli";
import {
  useAicliAccount,
  useAicliCachedValue,
  useAicliUnbind,
} from "@/hooks/useAicliAccount";
import { useOpenAicliBind } from "./AicliBindGate";

/** 解除绑定按顺序会发生的事：还原（青）、清除（红）、保留（灰）。 */
const UNBIND_STEPS = [
  {
    key: "stepRestore",
    Icon: RotateCcw,
    tone: "bg-action-soft text-action-text",
  },
  { key: "stepClear", Icon: Trash2, tone: "bg-danger-soft text-danger-text" },
  { key: "stepKeep", Icon: Check, tone: "bg-subtle text-fg-2" },
] as const;

/**
 * 设置 → 通用 最上面的「账户」（2026-11 改版，设计稿画板 12）：这台电脑绑定的 AICLI 账户、
 * 剩余额度，「换一个令牌」打开绑定对话框，「解除绑定」先确认（后端 C1-05：先还原全部接管，
 * 再清掉本机的令牌和账户数据，本机请求记录保留）。没绑定时不显示（绑定由 AicliBindGate 拦截）。
 */
export function AicliAccountSettings() {
  const { t, i18n } = useTranslation();
  const account = useAicliAccount().data;
  const bound = account?.state === "bound" || account?.state === "needs_rebind";
  const cached = useAicliCachedValue("account", bound).data;
  const openBind = useOpenAicliBind();
  const unbind = useAicliUnbind();
  const [confirming, setConfirming] = useState(false);
  const [deleteTokens, setDeleteTokens] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (!account || !bound) return null;

  const remaining = formatAicliQuota(
    readAccountQuota(cached?.value).remaining,
    account.quota_display,
    i18n.language,
  );
  const initial = Array.from(account.username ?? "?")[0]?.toUpperCase();

  const close = () => {
    if (unbind.isPending) return;
    setConfirming(false);
    setDeleteTokens(false);
  };

  const confirm = () => {
    setError(null);
    unbind.mutate(deleteTokens, {
      onSuccess: () => {
        setConfirming(false);
        setDeleteTokens(false);
      },
      onError: (failure) => {
        setConfirming(false);
        setError(
          t("aicli.accountSettings.unbindFailed", {
            message: isAicliCommandError(failure)
              ? failure.message
              : String(failure),
          }),
        );
      },
    });
  };

  return (
    <SettingsBlock title={t("aicli.accountSettings.title")}>
      <SettingsCard>
        <div
          data-testid="aicli-account-settings"
          className="flex flex-wrap items-center gap-x-4 gap-y-3 px-4 py-3.5"
        >
          <span
            aria-hidden="true"
            className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-action-soft text-section text-action-text"
          >
            {initial}
          </span>
          <div className="flex min-w-0 flex-1 flex-col">
            <span className="flex min-w-0 flex-wrap items-center gap-2">
              <span className="truncate text-strong text-fg-1">
                {account.username || t("aicli.account.noData")}
              </span>
              {account.user_group && (
                <span className="inline-flex h-5 items-center rounded-full bg-subtle px-2 text-badge text-fg-2">
                  {account.user_group}
                </span>
              )}
            </span>
            <span className="text-caption text-fg-2">
              {account.state === "needs_rebind"
                ? t("aicli.account.tokenInvalid")
                : t("aicli.accountSettings.boundHere", {
                    remaining: remaining ?? t("aicli.account.noData"),
                  })}
            </span>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button
              type="button"
              variant="neutral"
              size="regular"
              onClick={openBind}
            >
              {t("aicli.accountSettings.changeToken")}
            </Button>
            <Button
              type="button"
              variant="neutral"
              size="regular"
              className="text-danger-text hover:text-danger-text"
              disabled={unbind.isPending}
              onClick={() => setConfirming(true)}
            >
              {t("aicli.accountSettings.unbind")}
            </Button>
          </div>
        </div>
        <p className="m-0 border-t border-border px-4 py-2.5 text-caption text-fg-3">
          {t("aicli.accountSettings.unbindNote")}
        </p>
        {error && (
          <p
            role="alert"
            className="m-0 border-t border-border px-4 py-2.5 text-caption text-danger-text"
          >
            {error}
          </p>
        )}
      </SettingsCard>

      <Dialog open={confirming} onOpenChange={(open) => !open && close()}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("aicli.accountSettings.confirmTitle")}</DialogTitle>
          </DialogHeader>
          {/* 正文（2026-11 改版，设计稿画板 21）：按顺序会发生的三件事 + 可选项卡片 */}
          <div className="flex flex-col gap-4 px-6 pb-2">
            <DialogDescription className="text-body text-fg-2">
              {t("aicli.accountSettings.confirmBody")}
            </DialogDescription>
            <ul className="m-0 flex list-none flex-col gap-3 p-0">
              {UNBIND_STEPS.map(({ key, Icon, tone }) => (
                <li key={key} className="flex items-start gap-3">
                  <span
                    aria-hidden="true"
                    className={cn(
                      "flex h-8 w-8 shrink-0 items-center justify-center rounded-control",
                      tone,
                    )}
                  >
                    <Icon className="h-4 w-4" />
                  </span>
                  <span className="flex flex-col">
                    <span className="text-strong text-fg-1">
                      {t(`aicli.accountSettings.${key}`)}
                    </span>
                    <span className="text-caption text-fg-2">
                      {t(`aicli.accountSettings.${key}Body`)}
                    </span>
                  </span>
                </li>
              ))}
            </ul>
            <label className="flex cursor-pointer items-start gap-2.5 rounded-control border border-border px-3 py-2.5 text-body text-fg-1">
              <Checkbox
                checked={deleteTokens}
                disabled={unbind.isPending}
                onCheckedChange={(checked) => setDeleteTokens(checked === true)}
                className="mt-0.5"
              />
              <span className="flex flex-col gap-0.5">
                <span>{t("aicli.accountSettings.deleteTokens")}</span>
                <span className="text-caption text-fg-3">
                  {t("aicli.accountSettings.deleteTokensHint")}
                </span>
              </span>
            </label>
          </div>
          <DialogFooter>
            <Button
              type="button"
              variant="neutral"
              size="regular"
              disabled={unbind.isPending}
              onClick={close}
            >
              {t("common.cancel")}
            </Button>
            <Button
              type="button"
              variant="destructive"
              size="regular"
              disabled={unbind.isPending}
              onClick={confirm}
            >
              {unbind.isPending && (
                <Loader2
                  aria-hidden="true"
                  className="h-3.5 w-3.5 animate-spin"
                />
              )}
              {t("aicli.accountSettings.unbind")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </SettingsBlock>
  );
}
