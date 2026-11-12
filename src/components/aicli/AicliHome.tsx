import { useState } from "react";
import { useTranslation } from "react-i18next";
import { House } from "lucide-react";
import { AppPageHeader } from "@/components/shell/AppPageHeader";
import {
  useAicliAccount,
  useAicliCachedValue,
  useAicliPrivacy,
} from "@/hooks/useAicliAccount";
import { AicliOverviewPanel } from "./AicliOverviewPanel";
import { AicliProfilesPanel } from "./profiles/AicliProfilesPanel";

interface AicliHomeProps {
  /** 「管理配置集」入口；不传时在首页内打开配置集管理（F-05）。 */
  onOpenProfiles?: () => void;
  /** 隐私状态的「查看」入口（F-09 隐私脱敏页）。 */
  onOpenPrivacy?: () => void;
  /** 请求链路里「客户端」环节的入口（F-08 客户端总览）。 */
  onOpenClients?: () => void;
}

/**
 * 首页（F-03；2026-11 改版，设计稿画板 1）：AICLI 账户面板——额度、服务端每日消耗、
 * 请求链路（客户端 → 本地代理 → 隐私 → 线路 → 配置集 → 账户）和顶部异常提示。
 * 本机统计不再接在下面，是侧栏单独的「用量」页（负责人 2026-11-12 同意改 F-03/F-10 的口径）。
 * 未绑定时不会进到这里：绑定由 AicliBindGate 的对话框完成（F-07）。
 *
 * 状态规则：账户视图与服务端快照都来自本机缓存，有缓存时直接渲染，不显示整页
 * 加载态；后台刷新完成后由 aicli-data-updated 原地更新。
 */
export function AicliHome({
  onOpenProfiles,
  onOpenPrivacy,
  onOpenClients,
}: AicliHomeProps) {
  const { t } = useTranslation();
  const account = useAicliAccount().data;
  const bound = account?.state === "bound" || account?.state === "needs_rebind";
  const cached = useAicliCachedValue("account", bound);
  const privacy = useAicliPrivacy();
  const [profilesOpen, setProfilesOpen] = useState(false);

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <AppPageHeader
        icon={<House className="h-5 w-5" strokeWidth={1.5} />}
        title={t("nav.home")}
      />
      <div
        id="main-content"
        className="min-h-0 flex-1 overflow-y-auto scroll-stable px-6 pb-8 pt-2"
      >
        <div data-testid="aicli-home" className="flex flex-col gap-3">
          {account && bound && (
            <AicliOverviewPanel
              account={account}
              cached={cached.data}
              privacy={privacy.data}
              onOpenProfiles={onOpenProfiles ?? (() => setProfilesOpen(true))}
              onOpenPrivacy={onOpenPrivacy}
              onOpenClients={onOpenClients}
            />
          )}
        </div>
      </div>
      <AicliProfilesPanel
        isOpen={profilesOpen}
        onClose={() => setProfilesOpen(false)}
      />
    </div>
  );
}
