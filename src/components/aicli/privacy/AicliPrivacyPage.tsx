import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import {
  ChevronDown,
  Loader2,
  ShieldCheck,
  ShieldOff,
  Trash2,
  WandSparkles,
} from "lucide-react";
import { AppPageHeader } from "@/components/shell/AppPageHeader";
import { APP_DISPLAY_NAME, AppGlyph } from "@/components/shell/AppGlyph";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { Button } from "@/components/ui/button";
import { HelpTip } from "@/components/ui/help-tip";
import { Notice } from "@/components/ui/notice";
import { SegmentedControl } from "@/components/ui/segmented-control";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import type { AppId } from "@/lib/api";
import { APP_IDS } from "@/config/appConfig";
import { cn } from "@/lib/utils";
import { useAicliPrivacy, useAicliSetPrivacy } from "@/hooks/useAicliAccount";
import {
  useAicliPrivacyClearEvents,
  useAicliPrivacyEvents,
  useAicliPrivacyPreview,
  useAicliPrivacySummary,
} from "@/hooks/useAicliPrivacyRecords";
import type {
  AicliPrivacyCategory,
  AicliPrivacyCount,
  AicliPrivacyEvent,
} from "@/types/aicli";
import { MiniBars } from "../MiniBars";
import { AicliPrivacySettings } from "./AicliPrivacySettings";

type Range = 7 | 30;

const CATEGORIES: AicliPrivacyCategory[] = [
  "secret",
  "personal",
  "network",
  "custom",
];

/** 类别的颜色点（与图例一致）。 */
const CATEGORY_DOT: Record<AicliPrivacyCategory, string> = {
  secret: "bg-[var(--chart-usage)]",
  personal: "bg-success",
  network: "bg-warning",
  custom: "bg-fg-3",
};

/** 细分类别 → 四组（与后端 events::category_of 一致）。 */
export function privacyCategoryOf(kind: string): AicliPrivacyCategory {
  switch (kind) {
    case "API_KEY":
    case "SECRET":
    case "TOKEN":
    case "PASSWORD":
    case "DB_PASSWORD":
    case "PRIVATE_KEY":
    case "CREDENTIAL":
    case "KEY":
    case "AKIA":
    case "ASIA":
    case "LTAI":
    case "JWT":
      return "secret";
    case "EMAIL":
    case "PHONE":
    case "ID_CARD":
    case "BANK_CARD":
    case "IBAN":
    case "SSN":
    case "PASSPORT":
    case "BIRTHDAY":
    case "PERSON":
      return "personal";
    case "HOSTNAME":
    case "IP":
    case "MAC":
    case "USERNAME":
    case "BUCKET":
    case "SERIAL":
      return "network";
    default:
      return "custom";
  }
}

function compact(value: number, locale: string): string {
  return new Intl.NumberFormat(locale, {
    notation: value >= 10_000 ? "compact" : "standard",
    maximumFractionDigits: 1,
  }).format(value);
}

function appName(app: string | null, fallback: string): string {
  if (app && (APP_IDS as readonly string[]).includes(app)) {
    return APP_DISPLAY_NAME[app as AppId];
  }
  return fallback;
}

/**
 * 隐私脱敏页（F-09，参考 Magpie「观测 › 脱敏」）：请求离开这台电脑前在本机把密钥、
 * 个人信息换成占位符，回复里再换回来。页面展示没让上游看到的数量、按天趋势、按客户端
 * 分布、按类别筛选的请求记录、「试一试」预览和隐私设置。记录只存在本机，保留 30 天，
 * 不含原值。
 */
export function AicliPrivacyPage() {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;
  const privacy = useAicliPrivacy().data;
  const setPrivacy = useAicliSetPrivacy();
  const [range, setRange] = useState<Range>(7);
  const [category, setCategory] = useState<AicliPrivacyCategory | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  const summary = useAicliPrivacySummary(range).data;
  const events = useAicliPrivacyEvents(range, category);
  const clear = useAicliPrivacyClearEvents();

  const enabled = privacy?.privacy_enabled ?? false;
  const degraded = privacy?.engine?.degraded ?? 0;

  const toggle = (value: boolean) => {
    if (!privacy) return;
    setPrivacy.mutate({
      privacyEnabled: value,
      privacyFailureMode: privacy.privacy_failure_mode,
      allowUnscannedMedia: privacy.allow_unscanned_media,
    });
  };

  const items = useMemo(
    () => events.data?.pages.flatMap((page) => page.items) ?? [],
    [events.data],
  );

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <AppPageHeader
        icon={<ShieldCheck className="h-5 w-5" strokeWidth={1.5} />}
        title={t("nav.privacy")}
        titleExtra={
          <HelpTip title={t("nav.privacy")}>
            {t("aicli.privacyPage.help")}
          </HelpTip>
        }
        actions={
          <label className="flex items-center gap-2 text-body text-fg-2">
            {enabled ? t("aicli.privacyPage.on") : t("aicli.privacyPage.off")}
            <Switch
              aria-label={t("aicli.settings.privacyEnabled")}
              checked={enabled}
              disabled={!privacy?.bound || setPrivacy.isPending}
              onCheckedChange={toggle}
            />
          </label>
        }
      />
      <div
        id="main-content"
        data-testid="aicli-privacy-page"
        className="flex min-h-0 flex-1 flex-col gap-6 overflow-y-auto scroll-stable px-6 pb-10 pt-4"
      >
        {privacy && !enabled && (
          <Notice
            tone="neutral"
            icon={ShieldOff}
            title={t("aicli.privacyPage.offNotice")}
            actions={
              <Button
                type="button"
                variant="solid"
                size="compact"
                disabled={!privacy.bound || setPrivacy.isPending}
                onClick={() => toggle(true)}
              >
                {t("aicli.privacyPage.enable")}
              </Button>
            }
          />
        )}
        {enabled && degraded > 0 && (
          <Notice
            tone="danger"
            title={t("aicli.privacy.degraded", { count: degraded })}
          />
        )}
        {enabled && privacy && !privacy.engine && (
          <Notice tone="neutral" title={t("aicli.privacy.engineStopped")} />
        )}

        <Hero
          range={range}
          onRangeChange={setRange}
          summary={summary}
          locale={locale}
        />

        <section
          aria-labelledby="aicli-privacy-records"
          className="flex flex-col gap-3"
        >
          <div className="flex flex-wrap items-center gap-2">
            <h2
              id="aicli-privacy-records"
              className="m-0 me-1 text-caption font-semibold text-fg-2"
            >
              {t("aicli.privacyPage.recordsTitle")}
            </h2>
            <CategoryChip
              label={t("aicli.privacyPage.filterAll")}
              count={summary?.total_values ?? 0}
              active={category === null}
              locale={locale}
              onClick={() => setCategory(null)}
            />
            {CATEGORIES.map((key) => (
              <CategoryChip
                key={key}
                label={t(`aicli.privacyPage.categories.${key}`)}
                dot={CATEGORY_DOT[key]}
                count={
                  summary?.categories.find((item) => item.key === key)?.count ??
                  0
                }
                active={category === key}
                locale={locale}
                onClick={() => setCategory(key)}
              />
            ))}
          </div>
          <EventList
            items={items}
            loading={events.isPending}
            hasMore={Boolean(events.hasNextPage)}
            loadingMore={events.isFetchingNextPage}
            onLoadMore={() => void events.fetchNextPage()}
          />
        </section>

        <TryIt />

        <AicliPrivacySettings />

        <div className="flex flex-wrap items-center gap-2 text-caption text-fg-3">
          <span>
            {t("aicli.privacyPage.footer", {
              days: summary?.retention_days ?? 30,
            })}
          </span>
          <Button
            type="button"
            variant="quiet"
            size="compact"
            className="ms-auto"
            disabled={clear.isPending || items.length === 0}
            onClick={() => setConfirmClear(true)}
          >
            <Trash2 aria-hidden="true" className="h-3.5 w-3.5" />
            {t("aicli.privacyPage.clear")}
          </Button>
        </div>
      </div>
      <ConfirmDialog
        isOpen={confirmClear}
        title={t("aicli.privacyPage.clearConfirmTitle")}
        message={t("aicli.privacyPage.clearConfirmBody")}
        confirmText={t("aicli.privacyPage.clear")}
        variant="destructive"
        onConfirm={() => {
          setConfirmClear(false);
          clear.mutate();
        }}
        onCancel={() => setConfirmClear(false)}
      />
    </div>
  );
}

function Hero({
  range,
  onRangeChange,
  summary,
  locale,
}: {
  range: Range;
  onRangeChange: (range: Range) => void;
  summary: ReturnType<typeof useAicliPrivacySummary>["data"];
  locale: string;
}) {
  const { t } = useTranslation();
  const days = summary?.days ?? [];
  const today = days[days.length - 1]?.day;
  const apps = summary?.apps ?? [];
  const maxApp = Math.max(1, ...apps.map((app) => app.count));
  const total = summary?.total_values ?? 0;

  return (
    <section
      aria-labelledby="aicli-privacy-hero"
      data-testid="aicli-privacy-hero"
      className="overflow-hidden rounded-panel border border-border bg-surface"
    >
      <div className="grid gap-6 p-5 lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
        <div className="flex min-w-0 flex-col gap-3">
          <div className="flex items-start gap-3">
            <div className="flex min-w-0 flex-col">
              <h2 id="aicli-privacy-hero" className="m-0 text-strong text-fg-1">
                {t("aicli.privacyPage.heroTitle")}
              </h2>
              <p className="m-0 text-caption text-fg-3">
                {t("aicli.privacyPage.heroSubtitle")}
              </p>
            </div>
            <SegmentedControl<string>
              size="sm"
              className="ms-auto lg:hidden"
              aria-label={t("aicli.overview.rangeLabel")}
              value={String(range)}
              onValueChange={(value) => onRangeChange(Number(value) as Range)}
              items={[
                { value: "7", label: t("aicli.overview.range7") },
                { value: "30", label: t("aicli.overview.range30") },
              ]}
            />
          </div>
          <div className="flex flex-col">
            <span
              data-testid="aicli-privacy-total"
              title={total.toLocaleString(locale)}
              className="text-[44px] font-semibold leading-[52px] tabular-nums text-fg-1"
            >
              {compact(total, locale)}
            </span>
            <span className="text-body text-fg-2">
              {t("aicli.privacyPage.maskedUnit")}
            </span>
          </div>
          <p
            data-testid="aicli-privacy-distinct"
            className="m-0 flex flex-wrap gap-x-4 text-body text-fg-2"
          >
            <span>
              <strong className="font-semibold tabular-nums text-fg-1">
                {compact(summary?.distinct_values ?? 0, locale)}
              </strong>{" "}
              {t("aicli.privacyPage.distinct")}
            </span>
            <span>
              <strong className="font-semibold tabular-nums text-fg-1">
                {compact(summary?.requests ?? 0, locale)}
              </strong>{" "}
              {t("aicli.privacyPage.requests")}
            </span>
          </p>
          <MiniBars
            ariaLabel={t("aicli.privacyPage.trend")}
            height={56}
            startLabel={days[0]?.day.slice(5)}
            endLabel={t("aicli.overview.today")}
            bars={days.map((day) => ({
              key: day.day,
              label: day.day,
              value: day.values,
              display: t("aicli.privacyPage.dayValue", {
                values: day.values,
                requests: day.requests,
              }),
              highlight: day.day === today,
            }))}
          />
        </div>

        <div className="flex min-w-0 flex-col gap-3">
          <div className="hidden items-center lg:flex">
            <h3 className="m-0 text-caption font-semibold text-fg-2">
              {t("aicli.privacyPage.byClient")}
            </h3>
            <SegmentedControl<string>
              size="sm"
              className="ms-auto"
              aria-label={t("aicli.overview.rangeLabel")}
              value={String(range)}
              onValueChange={(value) => onRangeChange(Number(value) as Range)}
              items={[
                { value: "7", label: t("aicli.overview.range7") },
                { value: "30", label: t("aicli.overview.range30") },
              ]}
            />
          </div>
          {apps.length === 0 ? (
            <p className="m-0 flex flex-1 items-center justify-center rounded-panel border border-dashed border-border px-4 py-6 text-center text-caption text-fg-3">
              {t("aicli.privacyPage.noClients")}
            </p>
          ) : (
            <ul className="m-0 flex list-none flex-col gap-2.5 p-0">
              {apps.map((app) => (
                <li
                  key={app.key}
                  data-testid={`aicli-privacy-app-${app.key}`}
                  className="flex items-center gap-2.5"
                >
                  {(APP_IDS as readonly string[]).includes(app.key) ? (
                    <AppGlyph
                      app={app.key as AppId}
                      size={18}
                      badgeClassName="bg-surface"
                    />
                  ) : (
                    <span className="h-[20px] w-[20px] shrink-0 rounded-full bg-subtle" />
                  )}
                  <span className="w-28 shrink-0 truncate text-body text-fg-1">
                    {appName(app.key, t("aicli.privacyPage.otherClient"))}
                  </span>
                  <span className="h-2 flex-1 overflow-hidden rounded-full bg-subtle">
                    <span
                      className="block h-full rounded-full bg-[var(--chart-usage)]"
                      style={{ width: `${(app.count / maxApp) * 100}%` }}
                    />
                  </span>
                  <span className="w-14 text-end text-caption tabular-nums text-fg-2">
                    {compact(app.count, locale)}
                  </span>
                </li>
              ))}
            </ul>
          )}
          <dl className="m-0 mt-auto grid grid-cols-3 gap-2">
            {(
              [
                ["blocked", summary?.blocked ?? 0, "text-danger-text"],
                ["degraded", summary?.degraded ?? 0, "text-warning-text"],
                ["mediaPassed", summary?.media_passed ?? 0, "text-fg-1"],
              ] as const
            ).map(([key, value, tone]) => (
              <div
                key={key}
                className="flex flex-col rounded-control bg-subtle px-3 py-2"
              >
                <dt className="text-badge text-fg-3">
                  {t(`aicli.privacyPage.stats.${key}`)}
                </dt>
                <dd
                  className={cn(
                    "m-0 text-section tabular-nums",
                    value > 0 ? tone : "text-fg-1",
                  )}
                >
                  {value.toLocaleString(locale)}
                </dd>
              </div>
            ))}
          </dl>
        </div>
      </div>
    </section>
  );
}

function CategoryChip({
  label,
  count,
  dot,
  active,
  locale,
  onClick,
}: {
  label: string;
  count: number;
  dot?: string;
  active: boolean;
  locale: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={cn(
        "inline-flex h-7 items-center gap-1.5 rounded-full border px-3 text-caption transition-colors",
        active
          ? "border-transparent bg-[color-mix(in_srgb,var(--chart-usage)_14%,transparent)] font-medium text-[var(--chart-usage)]"
          : "border-border bg-surface text-fg-2 hover:border-border-strong",
      )}
    >
      {dot && (
        <span
          aria-hidden="true"
          className={cn("h-1.5 w-1.5 rounded-full", dot)}
        />
      )}
      {label}
      <span className="tabular-nums opacity-80">{compact(count, locale)}</span>
    </button>
  );
}

function KindChips({ kinds }: { kinds: AicliPrivacyCount[] }) {
  const { t } = useTranslation();
  return (
    <span className="flex flex-wrap justify-end gap-1">
      {kinds.slice(0, 3).map((kind) => (
        <span
          key={kind.key}
          className="inline-flex h-5 items-center gap-1 rounded-full border border-border px-2 text-badge text-fg-2"
        >
          <span
            aria-hidden="true"
            className={cn(
              "h-1.5 w-1.5 rounded-full",
              CATEGORY_DOT[privacyCategoryOf(kind.key)],
            )}
          />
          {t(`aicli.privacyPage.kinds.${kind.key}`, {
            defaultValue: kind.key,
          })}
          <span className="tabular-nums text-fg-3">×{kind.count}</span>
        </span>
      ))}
      {kinds.length > 3 && (
        <span className="inline-flex h-5 items-center rounded-full border border-border px-2 text-badge text-fg-3">
          +{kinds.length - 3}
        </span>
      )}
    </span>
  );
}

function dayHeading(ts: number, t: TFunction, locale: string): string {
  const date = new Date(ts);
  const today = new Date();
  const yesterday = new Date();
  yesterday.setDate(today.getDate() - 1);
  if (date.toDateString() === today.toDateString())
    return t("aicli.overview.today");
  if (date.toDateString() === yesterday.toDateString())
    return t("aicli.privacyPage.yesterday");
  return date.toLocaleDateString(locale, { month: "short", day: "numeric" });
}

function EventList({
  items,
  loading,
  hasMore,
  loadingMore,
  onLoadMore,
}: {
  items: AicliPrivacyEvent[];
  loading: boolean;
  hasMore: boolean;
  loadingMore: boolean;
  onLoadMore: () => void;
}) {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;

  if (loading) {
    return <div className="h-[160px] animate-pulse rounded-panel bg-subtle" />;
  }
  if (items.length === 0) {
    return (
      <div
        data-testid="aicli-privacy-empty"
        className="flex flex-col items-center gap-2 rounded-panel border border-dashed border-border px-6 py-10 text-center"
      >
        <span
          aria-hidden="true"
          className="flex h-11 w-11 items-center justify-center rounded-full bg-subtle text-fg-2"
        >
          <ShieldCheck className="h-5 w-5" strokeWidth={1.5} />
        </span>
        <p className="m-0 text-section text-fg-1">
          {t("aicli.privacyPage.emptyTitle")}
        </p>
        <p className="m-0 max-w-[420px] text-body text-fg-2">
          {t("aicli.privacyPage.emptyBody")}
        </p>
      </div>
    );
  }

  const groups: { heading: string; rows: AicliPrivacyEvent[] }[] = [];
  for (const item of items) {
    const heading = dayHeading(item.ts, t, locale);
    const last = groups[groups.length - 1];
    if (last && last.heading === heading) last.rows.push(item);
    else groups.push({ heading, rows: [item] });
  }

  return (
    <div className="flex flex-col gap-4">
      {groups.map((group) => (
        <div key={group.heading} className="flex flex-col gap-1.5">
          <h3 className="m-0 text-caption text-fg-3">{group.heading}</h3>
          <ul className="m-0 list-none overflow-hidden rounded-panel border border-border bg-surface p-0">
            {group.rows.map((item) => (
              <EventRow key={item.id} item={item} locale={locale} />
            ))}
          </ul>
        </div>
      ))}
      {hasMore && (
        <Button
          type="button"
          variant="neutral"
          size="compact"
          className="self-center"
          disabled={loadingMore}
          onClick={onLoadMore}
        >
          {loadingMore && (
            <Loader2 aria-hidden="true" className="h-3.5 w-3.5 animate-spin" />
          )}
          {t("aicli.privacyPage.loadMore")}
        </Button>
      )}
    </div>
  );
}

function EventRow({
  item,
  locale,
}: {
  item: AicliPrivacyEvent;
  locale: string;
}) {
  const { t } = useTranslation();
  const known =
    item.app_type && (APP_IDS as readonly string[]).includes(item.app_type);
  const outcomeTone: Record<string, string> = {
    blocked: "bg-danger-soft text-danger-text",
    degraded: "bg-warning-soft text-warning-text",
    media_passed: "bg-subtle text-fg-2",
  };
  return (
    <li
      data-testid={`aicli-privacy-event-${item.id}`}
      className="flex min-h-11 items-center gap-3 px-4 py-2 text-body [&:not(:last-child)]:border-b [&:not(:last-child)]:border-border"
    >
      <span className="w-12 shrink-0 tabular-nums text-fg-3">
        {new Date(item.ts).toLocaleTimeString(locale, {
          hour: "2-digit",
          minute: "2-digit",
        })}
      </span>
      <span className="flex min-w-0 flex-1 items-center gap-2">
        {known ? (
          <AppGlyph
            app={item.app_type as AppId}
            size={16}
            badgeClassName="bg-surface"
          />
        ) : null}
        <span className="truncate text-fg-1">
          {appName(item.app_type, t("aicli.privacyPage.otherClient"))}
        </span>
        {item.line_id && (
          <>
            <span aria-hidden="true" className="text-fg-3">
              →
            </span>
            <span className="truncate text-fg-2">
              {t("aicli.lines.aicliSource", {
                line: t(`aicli.lines.${item.line_id}`, {
                  defaultValue: item.line_id,
                }),
              })}
            </span>
          </>
        )}
        {item.outcome !== "masked" && (
          <span
            className={cn(
              "inline-flex h-5 shrink-0 items-center rounded-full px-2 text-badge font-medium",
              outcomeTone[item.outcome],
            )}
          >
            {t(`aicli.privacyPage.outcome.${item.outcome}`)}
          </span>
        )}
      </span>
      <KindChips kinds={item.kinds} />
      <span className="w-10 shrink-0 text-end font-semibold tabular-nums text-fg-1">
        {item.value_count > 0 ? item.value_count : "—"}
      </span>
    </li>
  );
}

/** 「试一试」：粘一段文字，看它离开这台电脑前会变成什么。 */
function TryIt() {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [text, setText] = useState("");
  const preview = useAicliPrivacyPreview();
  const result = preview.data;

  const highlighted = useMemo(() => {
    if (!result) return null;
    return result.masked
      .split(/(\{\{[A-Z][A-Z0-9_]*_[a-z2-7]{8}\}\})/)
      .map((part, index) =>
        /^\{\{[A-Z]/.test(part) ? (
          <mark
            key={index}
            className="rounded-[4px] bg-[color-mix(in_srgb,var(--chart-usage)_16%,transparent)] px-0.5 text-[var(--chart-usage)]"
          >
            {part}
          </mark>
        ) : (
          <span key={index}>{part}</span>
        ),
      );
  }, [result]);

  return (
    <section className="overflow-hidden rounded-panel border border-border bg-surface">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        className="flex w-full items-center gap-2 px-4 py-3 text-start transition-colors hover:bg-subtle"
      >
        <WandSparkles aria-hidden="true" className="h-4 w-4 text-fg-2" />
        <span className="flex min-w-0 flex-col">
          <span className="text-strong text-fg-1">
            {t("aicli.privacyPage.tryTitle")}
          </span>
          <span className="text-caption text-fg-3">
            {t("aicli.privacyPage.tryHint")}
          </span>
        </span>
        <ChevronDown
          aria-hidden="true"
          className={cn(
            "ms-auto h-4 w-4 text-fg-3 transition-transform",
            open && "rotate-180",
          )}
        />
      </button>
      {open && (
        <div className="flex flex-col gap-3 border-t border-border px-4 py-3">
          <Textarea
            aria-label={t("aicli.privacyPage.tryTitle")}
            placeholder={t("aicli.privacyPage.tryPlaceholder")}
            value={text}
            rows={4}
            onChange={(event) => setText(event.target.value)}
          />
          <div className="flex items-center gap-2">
            <Button
              type="button"
              variant="solid"
              size="compact"
              disabled={!text.trim() || preview.isPending}
              onClick={() => preview.mutate(text)}
            >
              {preview.isPending && (
                <Loader2
                  aria-hidden="true"
                  className="h-3.5 w-3.5 animate-spin"
                />
              )}
              {t("aicli.privacyPage.tryRun")}
            </Button>
            {result && (
              <span className="text-caption text-fg-2">
                {t("aicli.privacyPage.tryResult", { count: result.count })}
              </span>
            )}
          </div>
          {result && (
            <>
              <pre
                data-testid="aicli-privacy-preview"
                className="m-0 max-h-60 overflow-auto whitespace-pre-wrap break-all rounded-control bg-subtle px-3 py-2 font-mono text-caption text-fg-1"
              >
                {highlighted}
              </pre>
              {result.kinds.length > 0 && <KindChips kinds={result.kinds} />}
            </>
          )}
          {preview.error && (
            <p role="alert" className="m-0 text-caption text-danger-text">
              {t("aicli.privacyPage.tryFailed")}
            </p>
          )}
        </div>
      )}
    </section>
  );
}
