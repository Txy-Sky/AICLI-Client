import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Notice } from "@/components/ui/notice";
import { UsagePage } from "@/components/usage/UsagePage";
import {
  useAicliAccount,
  useAicliCachedValue,
  useAicliPrivacy,
} from "@/hooks/useAicliAccount";
import type { AppTypeFilter } from "@/types/usage";
import { AicliOverviewPanel } from "./AicliOverviewPanel";
import { useOpenAicliBind } from "./AicliBindGate";
import { AicliProfilesPanel } from "./profiles/AicliProfilesPanel";

interface AicliHomeProps {
  initialAppType?: AppTypeFilter;
  onOpenRoutingSettings?: () => void;
  /** 「管理配置集」入口；不传时在首页内打开配置集管理（F-05）。 */
  onOpenProfiles?: () => void;
  /** 隐私状态的「查看」入口（F-09 隐私脱敏页）。 */
  onOpenPrivacy?: () => void;
}

/**
 * 首页（F-03，作为 usage 页；F-10 改版）：AICLI 账户面板与本机统计在同一个滚动区，
 * 面板在上（额度、服务端每日消耗、线路、配置集、隐私），本机统计在下并标注
 * 「来源：本机记录」。未绑定时不会进到这里：绑定由 AicliBindGate 的对话框完成（F-07）。
 *
 * 状态规则：账户视图与服务端快照都来自本机缓存，有缓存时直接渲染，不显示整页
 * 加载态；后台刷新完成后由 aicli-data-updated 原地更新。
 */
export function AicliHome({
  initialAppType,
  onOpenRoutingSettings,
  onOpenProfiles,
  onOpenPrivacy,
}: AicliHomeProps) {
  const { t } = useTranslation();
  const accountQuery = useAicliAccount();
  const account = accountQuery.data;
  const bound = account?.state === "bound" || account?.state === "needs_rebind";
  const cached = useAicliCachedValue("account", bound);
  const privacy = useAicliPrivacy();
  const openBind = useOpenAicliBind();
  const [profilesOpen, setProfilesOpen] = useState(false);

  const leading = (
    <div data-testid="aicli-home" className="flex flex-col gap-3">
      {account && bound && account.state === "needs_rebind" && (
        <Notice
          tone="danger"
          title={t("aicli.account.tokenInvalid")}
          actions={
            <Button
              type="button"
              variant="neutral"
              size="compact"
              onClick={openBind}
            >
              {t("aicli.bind.rebindAction")}
            </Button>
          }
        />
      )}
      {account && bound && (
        <AicliOverviewPanel
          account={account}
          cached={cached.data}
          privacy={privacy.data}
          onOpenProfiles={onOpenProfiles ?? (() => setProfilesOpen(true))}
          onOpenPrivacy={onOpenPrivacy}
        />
      )}
    </div>
  );

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <UsagePage
        initialAppType={initialAppType}
        onOpenRoutingSettings={onOpenRoutingSettings}
        sourceNote={t("aicli.local.source")}
        localTitle={t("aicli.local.title")}
        leading={leading}
      />
      <AicliProfilesPanel
        isOpen={profilesOpen}
        onClose={() => setProfilesOpen(false)}
      />
    </div>
  );
}
