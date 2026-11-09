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
import { AicliAccountCard } from "./AicliAccountCard";
import { useOpenAicliBind } from "./AicliBindGate";
import { AicliLineSelector } from "./AicliLineSelector";
import { AicliProfileCard } from "./AicliProfileCard";
import { AicliProfilesPanel } from "./profiles/AicliProfilesPanel";

interface AicliHomeProps {
  initialAppType?: AppTypeFilter;
  onOpenRoutingSettings?: () => void;
  /** 「管理配置集」入口；不传时在首页内打开配置集管理（F-05）。 */
  onOpenProfiles?: () => void;
}

/**
 * 首页（F-03，作为 usage 页）：自上而下 账户、线路、配置集、本地统计。
 * 未绑定时不会进到这里：绑定由 AicliBindGate 的对话框完成（F-07）。
 *
 * 状态规则：账户视图与服务端快照都来自本机缓存，有缓存时直接渲染，不显示整页
 * 加载态；后台刷新完成后由 aicli-data-updated 原地更新。本地统计就是原来的用量页，
 * 标题标注「来源：本机记录」。
 */
export function AicliHome({
  initialAppType,
  onOpenRoutingSettings,
  onOpenProfiles,
}: AicliHomeProps) {
  const { t } = useTranslation();
  const accountQuery = useAicliAccount();
  const account = accountQuery.data;
  const bound = account?.state === "bound" || account?.state === "needs_rebind";
  const cached = useAicliCachedValue("account", bound);
  const privacy = useAicliPrivacy();
  const openBind = useOpenAicliBind();
  const [profilesOpen, setProfilesOpen] = useState(false);

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <div
        data-testid="aicli-home"
        className="flex max-h-[55%] shrink-0 flex-col gap-3 overflow-y-auto scroll-stable px-6 pb-2 pt-4"
      >
        {account && bound && (
          <>
            {account.state === "needs_rebind" && (
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
                onOpenProfiles={onOpenProfiles ?? (() => setProfilesOpen(true))}
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
      <AicliProfilesPanel
        isOpen={profilesOpen}
        onClose={() => setProfilesOpen(false)}
      />
    </div>
  );
}
