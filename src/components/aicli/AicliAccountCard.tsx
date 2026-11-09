import { useTranslation } from "react-i18next";
import { Badge } from "@/components/ui/badge";
import { Notice } from "@/components/ui/notice";
import { formatAicliQuota, readAccountQuota } from "@/lib/aicliQuota";
import type {
  AicliAccountView,
  AicliCachedValue,
  AicliPrivacyView,
} from "@/types/aicli";

interface AicliAccountCardProps {
  account: AicliAccountView;
  /** 服务端账户快照（缓存优先）；null = 从未取到。 */
  cached: AicliCachedValue | null | undefined;
  privacy?: AicliPrivacyView | null;
}

export function formatSyncTime(seconds: number, locale?: string): string {
  return new Date(seconds * 1000).toLocaleString(locale);
}

/**
 * 账户块（F-03）：账户名、用户组、剩余与已用额度，标注「来源：服务端」和最后
 * 同步时间。没有快照时显示「暂无数据」（不得显示 0）；同步失败或过期时显示
 * 缓存数据与时间，徽标「缓存数据」。
 */
export function AicliAccountCard({
  account,
  cached,
  privacy,
}: AicliAccountCardProps) {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;
  const quota = readAccountQuota(cached?.value);
  const display = account.quota_display;
  const remaining = formatAicliQuota(quota.remaining, display, locale);
  const used = formatAicliQuota(quota.used, display, locale);
  const fromCache = Boolean(cached && (cached.stale || cached.last_error));
  const noData = t("aicli.account.noData");

  return (
    <section
      aria-labelledby="aicli-account-title"
      className="flex min-w-0 flex-col gap-2 rounded-panel border border-border bg-surface p-4"
    >
      <div className="flex flex-wrap items-center gap-2">
        <h2 id="aicli-account-title" className="m-0 text-section text-fg-1">
          {t("aicli.account.title")}
        </h2>
        <span className="text-caption text-fg-3">
          {t("aicli.account.source")}
        </span>
        {fromCache && (
          <Badge variant="outline" className="h-5 px-1.5 text-badge">
            {t("aicli.account.cached")}
          </Badge>
        )}
      </div>
      <dl className="m-0 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-body">
        <dt className="text-fg-2">{t("aicli.account.title")}</dt>
        <dd className="m-0 truncate text-fg-1">{account.username || noData}</dd>
        <dt className="text-fg-2">{t("aicli.account.group")}</dt>
        <dd className="m-0 truncate text-fg-1">
          {account.user_group || noData}
        </dd>
        <dt className="text-fg-2">{t("aicli.account.remaining")}</dt>
        <dd
          className="m-0 tabular-nums text-fg-1"
          data-testid="aicli-quota-remaining"
        >
          {remaining ?? noData}
        </dd>
        <dt className="text-fg-2">{t("aicli.account.used")}</dt>
        <dd
          className="m-0 tabular-nums text-fg-1"
          data-testid="aicli-quota-used"
        >
          {used ?? noData}
        </dd>
      </dl>
      <p className="m-0 text-caption text-fg-3">
        {cached
          ? t("aicli.account.lastSync", {
              time: formatSyncTime(cached.fetched_at, locale),
            })
          : t("aicli.account.neverSynced")}
      </p>
      {privacy?.privacy_enabled && <AicliPrivacyStatus privacy={privacy} />}
    </section>
  );
}

/** 隐私开启时的引擎状态；可用性优先模式下发生降级时为红色提示。 */
export function AicliPrivacyStatus({ privacy }: { privacy: AicliPrivacyView }) {
  const { t } = useTranslation();
  const mode =
    privacy.privacy_failure_mode === "pass"
      ? t("aicli.privacy.modePass")
      : t("aicli.privacy.modeBlock");
  const engine = privacy.engine;
  const degraded = engine?.degraded ?? 0;
  return (
    <div className="flex flex-col gap-2">
      <Notice tone="neutral" title={`${t("aicli.privacy.enabled")} · ${mode}`}>
        {engine
          ? t("aicli.privacy.engineRunning", { count: engine.masked_values })
          : t("aicli.privacy.engineStopped")}
      </Notice>
      {degraded > 0 && (
        <Notice
          tone="danger"
          title={t("aicli.privacy.degraded", { count: degraded })}
        />
      )}
    </div>
  );
}
