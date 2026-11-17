import { useTranslation } from "react-i18next";
import { ChevronRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Notice } from "@/components/ui/notice";
import {
  SettingsBlock,
  SettingsCard,
  SettingsRow,
} from "@/components/settings/SettingsLayout";
import { APP_DISPLAY_NAME, AppGlyph } from "@/components/shell/AppGlyph";
import type { AppId } from "@/lib/api";
import { useAicliPrivacy } from "@/hooks/useAicliAccount";
import { useAicliLines } from "@/hooks/useAicliLines";
import { useAicliTakeoverStatus } from "@/hooks/useAicliProfiles";

/**
 * 设置 → 本地路由里的 AICLI 部分（F-06）：线路说明、凭据存储降级提示、接管中的
 * 客户端。隐私设置在 F-09 移到侧栏「隐私脱敏」页，这里只留入口。
 */
export function AicliRoutingSettings({
  onOpenApp,
  onOpenPrivacy,
}: {
  onOpenApp: (app: AppId) => void;
  /** F-09：隐私设置移到侧栏「隐私脱敏」页，这里只给入口。 */
  onOpenPrivacy?: () => void;
}) {
  const { t } = useTranslation();
  const privacy = useAicliPrivacy().data;
  const lines = useAicliLines().data;
  const takeovers = useAicliTakeoverStatus().data ?? [];
  const active = takeovers.filter((entry) => entry.state !== "released");

  return (
    <>
      <SettingsBlock title={t("aicli.settings.linesTitle")}>
        <SettingsCard>
          {(lines?.lines ?? []).map((line) => (
            <SettingsRow
              key={line.id}
              label={t(line.label_key)}
              description={t("aicli.settings.lineDetail", {
                transport: t(`aicli.lines.transport.${line.transport}`, {
                  defaultValue: line.transport,
                }),
                firstByte: line.first_byte_s,
                stream: line.stream_total_s,
                nonStream: line.non_stream_s,
              })}
              control={
                line.id === lines?.selected_line_id ? (
                  <span className="text-caption font-medium text-fg-1">
                    {t("aicli.lines.current")}
                  </span>
                ) : undefined
              }
            />
          ))}
        </SettingsCard>
        <p className="m-0 text-caption text-fg-3">
          {t("aicli.lines.switchNote")}
        </p>
      </SettingsBlock>

      <SettingsBlock title={t("aicli.settings.privacyTitle")}>
        {privacy?.secret_backend === "file_fallback" && (
          <Notice tone="warning" title={t("aicli.settings.secretFallback")} />
        )}
        <SettingsCard>
          <SettingsRow
            label={t("aicli.privacyPage.movedTitle")}
            description={t("aicli.privacyPage.movedHint")}
            control={
              onOpenPrivacy ? (
                <Button variant="quiet" size="compact" onClick={onOpenPrivacy}>
                  {t("aicli.privacyPage.open")}
                  <ChevronRight className="h-3.5 w-3.5" />
                </Button>
              ) : undefined
            }
          />
        </SettingsCard>
      </SettingsBlock>

      <SettingsBlock title={t("aicli.settings.takeoverTitle")}>
        {active.length === 0 ? (
          <div className="rounded-panel border border-dashed border-border px-4 py-3 text-caption text-fg-3">
            {t("aicli.settings.noTakeover")}
          </div>
        ) : (
          <SettingsCard>
            {active.map((entry) => {
              const app = entry.app as AppId;
              return (
                <div
                  key={entry.app}
                  className="flex items-center gap-3 px-4 py-2.5"
                >
                  <AppGlyph app={app} size={18} badgeClassName="bg-surface" />
                  <div className="min-w-0 flex-1 text-body font-medium text-fg-1">
                    {APP_DISPLAY_NAME[app] ?? entry.app}
                    <span className="ms-2 text-caption font-normal text-fg-2">
                      {entry.state === "active"
                        ? t("aicli.client.status.active")
                        : t("aicli.client.status.pending")}
                    </span>
                  </div>
                  <Button
                    variant="quiet"
                    size="compact"
                    onClick={() => onOpenApp(app)}
                  >
                    {t("routingSettings.goTo")}
                    <ChevronRight className="h-3.5 w-3.5" />
                  </Button>
                </div>
              );
            })}
          </SettingsCard>
        )}
      </SettingsBlock>
    </>
  );
}
