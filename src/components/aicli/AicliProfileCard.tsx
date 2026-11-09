import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Notice } from "@/components/ui/notice";
import { useAicliRoutes } from "@/hooks/useAicliProfiles";

interface AicliProfileCardProps {
  /** 账户上的当前配置集；为空时用推荐预设。 */
  activeProfileId: string | null;
  enabled?: boolean;
  /** 「管理配置集」入口（F-05 编辑页）；不传则不显示入口。 */
  onOpenProfiles?: () => void;
}

/** 配置集块（F-03）：当前配置集、可用模型数、需要修复时的警告、入口。 */
export function AicliProfileCard({
  activeProfileId,
  enabled = true,
  onOpenProfiles,
}: AicliProfileCardProps) {
  const { t } = useTranslation();
  const routes = useAicliRoutes(enabled);
  const view = routes.data;
  const id = activeProfileId ?? view?.active_profile_id ?? "recommended";
  const profile = view?.profiles.find((entry) => entry.id === id);
  const name =
    profile?.name ??
    (id === "recommended" ? t("aicli.profile.recommended") : id);
  const count = profile?.routes.length ?? 0;
  const repair = profile?.repair;

  return (
    <section
      aria-labelledby="aicli-profile-title"
      className="flex min-w-0 flex-col gap-2 rounded-panel border border-border bg-surface p-4"
    >
      <div className="flex items-center gap-2">
        <h2 id="aicli-profile-title" className="m-0 text-section text-fg-1">
          {t("aicli.profile.title")}
        </h2>
        {onOpenProfiles && (
          <Button
            type="button"
            variant="quiet"
            size="compact"
            className="ms-auto"
            onClick={onOpenProfiles}
          >
            {t("aicli.profile.manage")}
          </Button>
        )}
      </div>
      <p className="m-0 truncate text-body text-fg-1">{name}</p>
      <p className="m-0 text-caption text-fg-2">
        {profile && count > 0
          ? t("aicli.profile.models", { count })
          : t("aicli.profile.noModels")}
      </p>
      {repair?.needs_repair && (
        <Notice
          tone="warning"
          title={t("aicli.profile.needsRepair", {
            groups: repair.missing_groups.join("、"),
            count: repair.affected_models.length,
          })}
        />
      )}
    </section>
  );
}
