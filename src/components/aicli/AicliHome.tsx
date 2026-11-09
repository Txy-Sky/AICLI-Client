import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Notice } from "@/components/ui/notice";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { UsagePage } from "@/components/usage/UsagePage";
import {
  useAicliAccount,
  useAicliCachedValue,
  useAicliPrivacy,
} from "@/hooks/useAicliAccount";
import type { AppTypeFilter } from "@/types/usage";
import { AicliAccountCard } from "./AicliAccountCard";
import { AicliBindCard } from "./AicliBindCard";
import { AicliLineSelector } from "./AicliLineSelector";
import { AicliProfileCard } from "./AicliProfileCard";

interface AicliHomeProps {
  initialAppType?: AppTypeFilter;
  onOpenRoutingSettings?: () => void;
  /** 「管理配置集」入口；F-05 前不传。 */
  onOpenProfiles?: () => void;
  /**
   * 深链 aicli-open-bind 的计数：每变一次打开绑定界面（未绑定时聚焦绑定卡片，
   * 已绑定时弹出重新绑定对话框）。
   */
  bindRequest?: number;
}

/**
 * 首页（F-03，作为 usage 页）：自上而下 绑定 / 账户、线路、配置集、本地统计。
 *
 * 状态规则：账户视图与服务端快照都来自本机缓存，有缓存时直接渲染，不显示整页
 * 加载态；后台刷新完成后由 aicli-data-updated 原地更新。本地统计就是原来的用量页，
 * 标题标注「来源：本机记录」。
 */
export function AicliHome({
  initialAppType,
  onOpenRoutingSettings,
  onOpenProfiles,
  bindRequest,
}: AicliHomeProps) {
  const { t } = useTranslation();
  const accountQuery = useAicliAccount();
  const account = accountQuery.data;
  const bound = account?.state === "bound" || account?.state === "needs_rebind";
  const cached = useAicliCachedValue("account", bound);
  const privacy = useAicliPrivacy();
  const [rebindOpen, setRebindOpen] = useState(false);

  useEffect(() => {
    if (bindRequest && bound) setRebindOpen(true);
    // 只在深链计数变化时响应；绑定状态变化不应重新弹窗
  }, [bindRequest]);

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <div
        data-testid="aicli-home"
        className="flex max-h-[55%] shrink-0 flex-col gap-3 overflow-y-auto scroll-stable px-6 pb-2 pt-4"
      >
        {account && !bound && (
          <AicliBindCard
            line={account.selected_line_id ?? "main"}
            focusSignal={bindRequest}
          />
        )}
        {account && bound && (
          <>
            {account.state === "needs_rebind" && (
              <>
                <Notice tone="danger" title={t("aicli.account.tokenInvalid")} />
                <AicliBindCard
                  line={account.selected_line_id ?? "main"}
                  focusSignal={bindRequest}
                />
              </>
            )}
            <div className="grid grid-cols-1 gap-3 lg:grid-cols-3">
              <AicliAccountCard
                account={account}
                cached={cached.data}
                privacy={privacy.data}
              />
              <AicliLineSelector enabled={bound} />
              <AicliProfileCard
                activeProfileId={account.active_profile_id}
                enabled={bound}
                onOpenProfiles={onOpenProfiles}
              />
            </div>
          </>
        )}
      </div>
      <UsagePage
        initialAppType={initialAppType}
        onOpenRoutingSettings={onOpenRoutingSettings}
        sourceNote={t("aicli.local.source")}
      />
      <Dialog open={rebindOpen} onOpenChange={setRebindOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("aicli.bind.rebindTitle")}</DialogTitle>
          </DialogHeader>
          <AicliBindCard
            line={account?.selected_line_id ?? "main"}
            focusSignal={bindRequest}
            onBound={() => setRebindOpen(false)}
            className="flex flex-col gap-3"
          />
        </DialogContent>
      </Dialog>
    </div>
  );
}
