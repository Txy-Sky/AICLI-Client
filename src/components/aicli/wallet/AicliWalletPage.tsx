import { useEffect, useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Check, Copy, ExternalLink, Loader2, Wallet } from "lucide-react";
import { AppPageHeader } from "@/components/shell/AppPageHeader";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { WEBSITE_URL } from "@/config/branding";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Notice } from "@/components/ui/notice";
import {
  formatSyncTime,
  lastDays,
  periodTotals,
} from "@/components/aicli/AicliOverviewPanel";
import { useAicliAccount, useAicliCachedValue } from "@/hooks/useAicliAccount";
import { useAicliUsageSummary } from "@/hooks/useAicliUsage";
import {
  useAicliAffTransfer,
  useAicliPayWatch,
  useAicliSubscriptionBuy,
  useAicliWallet,
  useAicliWalletHistory,
  useAicliWalletPay,
  useAicliWalletQuote,
  useAicliWalletRedeem,
  type AicliPayWatch,
} from "@/hooks/useAicliWallet";
import { useDebouncedValue } from "@/hooks/useDebouncedValue";
import { aicliErrorCode } from "@/lib/api/aicli";
import { settingsApi } from "@/lib/api/settings";
import { formatAicliQuota, readAccountQuota } from "@/lib/aicliQuota";
import {
  BALANCE_METHOD,
  discountPercent,
  inviteLink,
  minAffTransfer,
  parseTopupAmount,
  payMethodMark,
  planDuration,
  planPriceQuota,
  quotaPerUnit,
  readAffStats,
  topupInputSymbol,
  topupQuota,
  walletErrorText,
} from "@/lib/aicliWallet";
import { copyText } from "@/lib/clipboard";
import { toast } from "@/lib/toast";
import { cn } from "@/lib/utils";
import type {
  AicliPayKind,
  AicliPayMethod,
  AicliPayStarted,
  AicliPlan,
  AicliQuotaDisplay,
  AicliSubscription,
  AicliTopupInfo,
} from "@/types/aicli";

const HISTORY_PAGE_SIZE = 10;
const QUOTE_DEBOUNCE_MS = 400;
/** 余额卡的日均与可用天数按近 7 天算（与首页默认区间一致）。 */
const RUNWAY_DAYS = 7;
const RESET_PERIODS = ["daily", "weekly", "monthly"];
const STATUS_TONE: Record<string, string> = {
  success: "bg-success-soft text-success-text",
  pending: "bg-warning-soft text-warning-text",
};

type Display = AicliQuotaDisplay | null | undefined;
type StartPay = (
  kind: AicliPayKind,
  started: AicliPayStarted,
  subscriptionBaseline?: number,
) => void;

const panelClass =
  "flex min-w-0 flex-col gap-4 rounded-panel border border-border bg-surface p-5";

function PanelTitle({ id, children }: { id: string; children: ReactNode }) {
  return (
    <h2 id={id} className="m-0 text-section text-fg-1">
      {children}
    </h2>
  );
}

/** 一组小标签 + 数值（余额卡、邀请统计）。 */
function Stat({
  label,
  value,
  testId,
}: {
  label: string;
  value: string;
  testId?: string;
}) {
  return (
    <div className="flex min-w-0 flex-col">
      <span className="text-caption text-fg-2">{label}</span>
      <span
        data-testid={testId}
        className="truncate text-section tabular-nums text-fg-1"
      >
        {value}
      </span>
    </div>
  );
}

/** 支付方式的色块标记（图形不读出，名字由按钮文字给出）。 */
function MethodMark({ method }: { method: AicliPayMethod }) {
  const mark = payMethodMark(method.type, method.name);
  return (
    <span
      aria-hidden="true"
      style={{ backgroundColor: mark.color }}
      className="inline-flex h-[22px] w-[22px] shrink-0 items-center justify-center rounded-control text-caption font-bold text-white"
    >
      {mark.letter}
    </span>
  );
}

function choiceClass(selected: boolean) {
  return cn(
    "rounded-control border text-fg-1 transition-colors disabled:cursor-not-allowed disabled:opacity-50",
    selected
      ? "border-action bg-action-soft ring-1 ring-action"
      : "border-border-strong bg-surface hover:bg-subtle",
  );
}

/** 等待支付 / 到账结果的提示条。 */
function PayStatus({
  watch,
  onDismiss,
}: {
  watch: AicliPayWatch;
  onDismiss: () => void;
}) {
  const { t } = useTranslation();
  if (watch.phase === "idle") return null;
  if (watch.phase === "waiting") {
    return (
      <div role="status" data-testid="aicli-wallet-pay-waiting">
        <Notice
          tone="route"
          icon={Loader2}
          title={
            watch.windowOpen
              ? t("aicli.wallet.pay.waitingTitle")
              : t("aicli.wallet.pay.confirmingTitle")
          }
          actions={
            <Button
              type="button"
              variant="neutral"
              size="compact"
              onClick={onDismiss}
            >
              {t("aicli.wallet.pay.stop")}
            </Button>
          }
        >
          {watch.windowOpen
            ? t("aicli.wallet.pay.waitingBody", { host: watch.host ?? "" })
            : t("aicli.wallet.pay.confirmingBody")}
        </Notice>
      </div>
    );
  }
  if (watch.result === "success") {
    return (
      <div role="status" data-testid="aicli-wallet-pay-success">
        <Notice
          tone="direct"
          title={
            watch.kind === "topup"
              ? t("aicli.wallet.pay.successTopup")
              : t("aicli.wallet.pay.successSubscription")
          }
          onDismiss={onDismiss}
          dismissLabel={t("aicli.wallet.pay.dismiss")}
        />
      </div>
    );
  }
  return (
    <div role="status" data-testid="aicli-wallet-pay-not-detected">
      <Notice
        tone="warning"
        title={t("aicli.wallet.pay.notDetectedTitle")}
        onDismiss={onDismiss}
        dismissLabel={t("aicli.wallet.pay.dismiss")}
      >
        {t("aicli.wallet.pay.notDetectedBody")}
      </Notice>
    </div>
  );
}

function BalanceCard({
  remaining,
  used,
  display,
  locale,
}: {
  remaining: number | null;
  used: number | null;
  display: Display;
  locale: string;
}) {
  const { t } = useTranslation();
  const noData = t("aicli.account.noData");
  const fmt = (value: number | null | undefined) =>
    formatAicliQuota(value, display, locale);
  const totals = periodTotals(
    useAicliUsageSummary(true, 30).data,
    lastDays(RUNWAY_DAYS),
  );
  const dailyAvg = totals ? totals.quota / RUNWAY_DAYS : null;
  const runway =
    dailyAvg != null && dailyAvg > 0 && remaining != null && remaining > 0
      ? Math.floor(remaining / dailyAvg)
      : null;

  return (
    <section
      aria-label={t("aicli.account.remaining")}
      className="flex min-w-0 flex-col gap-3.5 rounded-panel bg-action-soft p-5"
    >
      <div className="flex min-w-0 flex-col gap-0.5">
        <span className="text-caption text-fg-2">
          {t("aicli.account.remaining")}
        </span>
        <span
          data-testid="aicli-wallet-balance"
          className="truncate text-[40px] font-bold leading-[48px] tracking-[-0.5px] tabular-nums text-fg-1"
        >
          {fmt(remaining) ?? noData}
        </span>
      </div>
      <div className="flex flex-wrap gap-x-6 gap-y-2">
        <Stat label={t("aicli.account.used")} value={fmt(used) ?? noData} />
        <Stat
          label={t("aicli.wallet.balance.dailyAvg", { days: RUNWAY_DAYS })}
          value={fmt(dailyAvg) ?? noData}
        />
      </div>
      {runway != null ? (
        <span
          data-testid="aicli-wallet-runway"
          className="border-t border-[color-mix(in_srgb,var(--action-bg)_25%,transparent)] pt-3 text-caption text-action-text"
        >
          {t("aicli.overview.runway", { days: RUNWAY_DAYS, count: runway })}
        </span>
      ) : null}
    </section>
  );
}

/** 一档预设：数量 + 这一档的实付金额（服务端计算）。 */
function PresetTile({
  amount,
  selected,
  discount,
  display,
  locale,
  onSelect,
}: {
  amount: number;
  selected: boolean;
  discount: number | null;
  display: Display;
  locale: string;
  onSelect: () => void;
}) {
  const { t } = useTranslation();
  const quote = useAicliWalletQuote(amount);
  return (
    <button
      type="button"
      aria-pressed={selected}
      onClick={onSelect}
      className={cn(
        choiceClass(selected),
        "relative flex h-16 flex-col items-center justify-center gap-0.5 px-2",
      )}
    >
      <span className="text-[16px] font-semibold leading-[22px] tabular-nums">
        {formatAicliQuota(topupQuota(amount, display), display, locale)}
      </span>
      <span className="text-caption tabular-nums text-fg-2">
        {quote.data != null
          ? t("aicli.wallet.topup.tileMoney", { money: quote.data })
          : " "}
      </span>
      {discount != null ? (
        <span className="absolute -top-2.5 end-2 rounded-full bg-success-text px-1.5 text-[11px] font-semibold leading-[18px] text-white">
          {t("aicli.wallet.topup.discount", { percent: discount })}
        </span>
      ) : null}
    </button>
  );
}

/** 结算小票的一行：标签在左、数值在右，中间虚线相连。 */
function ReceiptRow({
  label,
  children,
  strong = false,
}: {
  label: string;
  children: ReactNode;
  strong?: boolean;
}) {
  return (
    <div className="flex items-baseline gap-3">
      <dt
        className={cn(
          "shrink-0 text-body",
          strong ? "font-semibold text-fg-1" : "text-fg-2",
        )}
      >
        {label}
      </dt>
      <span
        aria-hidden="true"
        className="min-w-4 flex-1 border-b border-dashed border-border-strong"
      />
      <dd
        className={cn(
          "m-0 min-w-0 truncate tabular-nums",
          strong
            ? "text-[28px] font-bold leading-9 text-action-text"
            : "text-section text-fg-1",
        )}
      >
        {children}
      </dd>
    </div>
  );
}

function TopupPanel({
  info,
  display,
  locale,
  busy,
  onStart,
}: {
  info: AicliTopupInfo;
  display: Display;
  locale: string;
  /** 已有一笔支付在等待：不能再下单。 */
  busy: boolean;
  onStart: StartPay;
}) {
  const { t } = useTranslation();
  const pay = useAicliWalletPay();
  const [preset, setPreset] = useState<number | null>(
    () => info.amount_options[0] ?? null,
  );
  const [customText, setCustomText] = useState(() =>
    info.amount_options.length === 0 ? String(info.min_topup) : "",
  );
  const [method, setMethod] = useState(info.methods[0]?.type ?? "");
  // 支付方式列表变了（管理员改了配置）而当前选择不在其中时，回到第一项
  useEffect(() => {
    if (!info.methods.some((entry) => entry.type === method)) {
      setMethod(info.methods[0]?.type ?? "");
    }
  }, [info.methods, method]);

  const usingCustom = customText.trim() !== "";
  const amount = usingCustom ? parseTopupAmount(customText) : preset;
  const selected = info.methods.find((entry) => entry.type === method);
  const minimum = Math.max(info.min_topup, selected?.min_topup ?? 0);
  const belowMinimum = amount != null && amount < minimum;
  const valid = amount != null && !belowMinimum && Boolean(selected);
  const quoteAmount = useDebouncedValue(
    valid ? amount : null,
    QUOTE_DEBOUNCE_MS,
  );
  const quote = useAicliWalletQuote(quoteAmount);
  const money = valid && quoteAmount === amount ? quote.data : undefined;
  const fmtAmount = (value: number) =>
    formatAicliQuota(topupQuota(value, display), display, locale) ?? "";
  const symbol = topupInputSymbol(display);
  const dash = t("aicli.wallet.topup.pending");

  const submit = () => {
    if (!valid || amount == null) return;
    pay.mutate(
      { amount, method },
      {
        onSuccess: (started) => onStart("topup", started),
        onError: (error) => toast.error(walletErrorText(error, t)),
      },
    );
  };

  return (
    <section
      aria-labelledby="aicli-wallet-topup-title"
      data-testid="aicli-wallet-topup"
      className={cn(panelClass, "gap-[18px]")}
    >
      <PanelTitle id="aicli-wallet-topup-title">
        {t("aicli.wallet.topup.title")}
      </PanelTitle>

      <div className="flex flex-col gap-2">
        <span className="text-caption text-fg-2">
          {t("aicli.wallet.topup.amountLabel")}
        </span>
        {info.amount_options.length > 0 ? (
          <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
            {info.amount_options.map((option) => (
              <PresetTile
                key={option}
                amount={option}
                selected={!usingCustom && preset === option}
                discount={discountPercent(info.discounts, option)}
                display={display}
                locale={locale}
                onSelect={() => {
                  setPreset(option);
                  setCustomText("");
                }}
              />
            ))}
          </div>
        ) : null}
        <div
          className={cn(
            "flex h-12 min-w-0 items-center gap-2.5 rounded-control border bg-surface px-3.5 transition-[border-color,box-shadow] focus-within:border-ring focus-within:ring-[3px] focus-within:ring-ring/20",
            usingCustom && !valid ? "border-danger" : "border-border-strong",
            usingCustom && valid && "border-action ring-1 ring-action",
          )}
        >
          <label
            htmlFor="aicli-wallet-custom-amount"
            className="shrink-0 text-body text-fg-2"
          >
            {t("aicli.wallet.topup.customLabel")}
          </label>
          <span aria-hidden="true" className="h-5 w-px shrink-0 bg-border" />
          {symbol ? (
            <span
              aria-hidden="true"
              className="text-[16px] font-semibold text-fg-3"
            >
              {symbol}
            </span>
          ) : null}
          <input
            id="aicli-wallet-custom-amount"
            inputMode="numeric"
            autoComplete="off"
            placeholder={t("aicli.wallet.topup.customPlaceholder")}
            aria-invalid={usingCustom && !valid}
            aria-describedby="aicli-wallet-custom-hint"
            value={customText}
            onChange={(event) => setCustomText(event.target.value)}
            className="h-full min-w-0 flex-1 border-0 bg-transparent p-0 text-[16px] font-semibold tabular-nums text-fg-1 outline-none placeholder:font-normal placeholder:text-fg-3"
          />
          <span
            id="aicli-wallet-custom-hint"
            className="hidden shrink-0 text-caption text-fg-3 md:inline"
          >
            {t("aicli.wallet.topup.customHint", { min: fmtAmount(minimum) })}
          </span>
        </div>
        {belowMinimum ? (
          <span role="alert" className="text-caption text-danger-text">
            {t("aicli.wallet.topup.minHint", { min: fmtAmount(minimum) })}
          </span>
        ) : null}
      </div>

      <div className="flex flex-col gap-2">
        <span className="text-caption text-fg-2">
          {t("aicli.wallet.topup.methodLabel")}
        </span>
        <div
          role="group"
          aria-label={t("aicli.wallet.topup.methodLabel")}
          className="flex flex-wrap gap-2"
        >
          {info.methods.map((entry) => (
            <button
              key={entry.type}
              type="button"
              aria-pressed={entry.type === method}
              onClick={() => setMethod(entry.type)}
              className={cn(
                choiceClass(entry.type === method),
                "inline-flex h-11 items-center gap-2 px-4 text-body font-medium",
              )}
            >
              <MethodMark method={entry} />
              {entry.name}
            </button>
          ))}
        </div>
      </div>

      <div className="mt-auto flex flex-col gap-4 rounded-panel bg-sidebar px-5 py-[18px]">
        <dl
          data-testid="aicli-wallet-summary"
          className="m-0 flex flex-col gap-2.5"
        >
          <ReceiptRow label={t("aicli.wallet.topup.arriveLabel")}>
            {amount != null ? fmtAmount(amount) : dash}
          </ReceiptRow>
          <ReceiptRow label={t("aicli.wallet.topup.methodLabel")}>
            {selected?.name ?? dash}
          </ReceiptRow>
          <ReceiptRow label={t("aicli.wallet.topup.payLabel")} strong>
            {money != null ? t("aicli.wallet.topup.money", { money }) : dash}
          </ReceiptRow>
        </dl>
        <Button
          type="button"
          variant="solid"
          data-testid="aicli-wallet-pay-button"
          className="h-12 w-full rounded-[8px] text-[15px] font-bold"
          disabled={!valid || busy || pay.isPending}
          onClick={submit}
        >
          {busy
            ? t("aicli.wallet.topup.paying")
            : money != null
              ? t("aicli.wallet.topup.payWithMoney", { money })
              : t("aicli.wallet.topup.pay")}
        </Button>
      </div>
      <p className="m-0 text-caption text-fg-3">
        {t("aicli.wallet.topup.checkoutHint")}
      </p>
    </section>
  );
}

function TopupOffPanel({ link }: { link: string | null }) {
  const { t } = useTranslation();
  return (
    <section
      aria-labelledby="aicli-wallet-topup-off-title"
      data-testid="aicli-wallet-topup-off"
      className={panelClass}
    >
      <PanelTitle id="aicli-wallet-topup-off-title">
        {t("aicli.wallet.topup.title")}
      </PanelTitle>
      <p className="m-0 text-body text-fg-2">
        {t("aicli.wallet.topup.disabledBody")}
      </p>
      <Button
        type="button"
        variant="neutral"
        size="regular"
        className="gap-1.5 self-start"
        onClick={() => void settingsApi.openExternal(link ?? WEBSITE_URL)}
      >
        <ExternalLink
          aria-hidden="true"
          className="h-4 w-4"
          strokeWidth={1.5}
        />
        {t("aicli.wallet.topup.openWebsite")}
      </Button>
    </section>
  );
}

function RedeemPanel({
  display,
  locale,
  stacked,
}: {
  display: Display;
  locale: string;
  /** 放在左栏时输入框与按钮上下排。 */
  stacked: boolean;
}) {
  const { t } = useTranslation();
  const redeem = useAicliWalletRedeem();
  const [key, setKey] = useState("");
  const submit = () => {
    const value = key.trim();
    if (!value) return;
    redeem.mutate(value, {
      onSuccess: (quota) => {
        setKey("");
        toast.success(
          t("aicli.wallet.redeem.success", {
            value: formatAicliQuota(quota, display, locale) ?? "",
          }),
        );
      },
      onError: (error) => toast.error(walletErrorText(error, t)),
    });
  };
  return (
    <section
      aria-labelledby="aicli-wallet-redeem-title"
      data-testid="aicli-wallet-redeem"
      className={cn(panelClass, "flex-1 gap-3")}
    >
      <PanelTitle id="aicli-wallet-redeem-title">
        {t("aicli.wallet.redeem.title")}
      </PanelTitle>
      <p className="m-0 text-caption text-fg-2">
        {t("aicli.wallet.redeem.body")}
      </p>
      <form
        className={cn("flex gap-2", stacked ? "flex-col" : "items-center")}
        onSubmit={(event) => {
          event.preventDefault();
          submit();
        }}
      >
        <Input
          aria-label={t("aicli.wallet.redeem.title")}
          placeholder={t("aicli.wallet.redeem.placeholder")}
          className="h-9 min-w-0 flex-1 font-mono"
          value={key}
          onChange={(event) => setKey(event.target.value)}
        />
        <Button
          type="submit"
          variant="neutral"
          size="regular"
          className={cn("h-9 px-5", stacked && "self-start")}
          disabled={!key.trim() || redeem.isPending}
        >
          {t("aicli.wallet.redeem.submit")}
        </Button>
      </form>
    </section>
  );
}

function MySubscriptions({
  active,
  plans,
  display,
  locale,
}: {
  active: AicliSubscription[];
  plans: AicliPlan[];
  display: Display;
  locale: string;
}) {
  const { t } = useTranslation();
  const fmt = (quota: number) => formatAicliQuota(quota, display, locale) ?? "";
  return (
    <section
      aria-labelledby="aicli-wallet-mine-title"
      data-testid="aicli-wallet-mine"
      className={cn(panelClass, "flex-1 gap-3")}
    >
      <PanelTitle id="aicli-wallet-mine-title">
        {t("aicli.wallet.plans.mine")}
      </PanelTitle>
      {active.map((entry) => {
        const plan = plans.find((candidate) => candidate.id === entry.plan_id);
        const share =
          entry.amount_total > 0
            ? Math.min(1, Math.max(0, entry.amount_used / entry.amount_total))
            : null;
        return (
          <div
            key={entry.id}
            className="flex flex-col gap-3 [&:not(:last-child)]:border-b [&:not(:last-child)]:border-border [&:not(:last-child)]:pb-3"
          >
            <div className="flex items-start gap-2">
              <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <span className="truncate text-section text-fg-1">
                  {plan?.title ||
                    t("aicli.wallet.plans.planFallback", { id: entry.plan_id })}
                </span>
                <span className="text-caption text-fg-2">
                  {[
                    entry.end_time > 0
                      ? t("aicli.wallet.plans.expiresOn", {
                          time: new Date(
                            entry.end_time * 1000,
                          ).toLocaleDateString(locale),
                        })
                      : null,
                    plan && RESET_PERIODS.includes(plan.quota_reset_period)
                      ? t(`aicli.wallet.plans.reset.${plan.quota_reset_period}`)
                      : null,
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                </span>
              </div>
              <span className="shrink-0 rounded-full bg-success-soft px-2 text-caption font-semibold text-success-text">
                {t("aicli.wallet.plans.active")}
              </span>
            </div>
            {share != null ? (
              <div className="flex flex-col gap-1.5">
                <div
                  role="meter"
                  aria-label={t("aicli.wallet.plans.usedShare")}
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuenow={Math.round(share * 100)}
                  className="h-1.5 overflow-hidden rounded-full bg-subtle"
                >
                  <div
                    className="h-full bg-action"
                    style={{ width: `${share * 100}%` }}
                  />
                </div>
                <div className="flex justify-between text-caption tabular-nums text-fg-2">
                  <span>
                    {t("aicli.wallet.plans.usedValue", {
                      value: fmt(entry.amount_used),
                    })}
                  </span>
                  <span>
                    {t("aicli.wallet.plans.totalValue", {
                      value: fmt(entry.amount_total),
                    })}
                  </span>
                </div>
              </div>
            ) : (
              <span className="text-caption text-fg-2">
                {t("aicli.wallet.plans.unlimited")}
              </span>
            )}
          </div>
        );
      })}
    </section>
  );
}

function PlansPanel({
  plans,
  subscriptions,
  methods,
  display,
  locale,
  busy,
  onStart,
}: {
  plans: AicliPlan[];
  subscriptions: AicliSubscription[];
  methods: AicliPayMethod[];
  display: Display;
  locale: string;
  busy: boolean;
  onStart: StartPay;
}) {
  const { t } = useTranslation();
  const buy = useAicliSubscriptionBuy();
  const [choosing, setChoosing] = useState<AicliPlan | null>(null);
  const fmt = (quota: number) => formatAicliQuota(quota, display, locale) ?? "";
  const activePlanIds = new Set(
    subscriptions
      .filter((entry) => entry.status === "active")
      .map((entry) => entry.plan_id),
  );

  const purchase = (plan: AicliPlan, method: string) => {
    buy.mutate(
      { planId: plan.id, method },
      {
        onSuccess: (started) => {
          setChoosing(null);
          if (started.completed) {
            toast.success(t("aicli.wallet.plans.bought"));
          } else {
            onStart("subscription", started, subscriptions.length);
          }
        },
        onError: (error) => toast.error(walletErrorText(error, t)),
      },
    );
  };

  return (
    <section
      aria-labelledby="aicli-wallet-plans-title"
      data-testid="aicli-wallet-plans"
      className={panelClass}
    >
      <PanelTitle id="aicli-wallet-plans-title">
        {t("aicli.wallet.plans.title")}
      </PanelTitle>
      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
        {plans.map((plan) => {
          const current = activePlanIds.has(plan.id);
          const features = [
            (plan.total_amount > 0
              ? t("aicli.wallet.plans.quota", { value: fmt(plan.total_amount) })
              : t("aicli.wallet.plans.unlimited")) +
              (RESET_PERIODS.includes(plan.quota_reset_period)
                ? ` · ${t(`aicli.wallet.plans.reset.${plan.quota_reset_period}`)}`
                : ""),
            plan.upgrade_group
              ? t("aicli.wallet.plans.group", { group: plan.upgrade_group })
              : null,
            plan.max_purchase_per_user > 0
              ? t("aicli.wallet.plans.limit", {
                  count: plan.max_purchase_per_user,
                })
              : null,
          ].filter((item): item is string => Boolean(item));
          return (
            <article
              key={plan.id}
              data-testid={`aicli-wallet-plan-${plan.id}`}
              className={cn(
                "flex min-w-0 flex-col gap-3 rounded-panel border p-4",
                current ? "border-action ring-1 ring-action" : "border-border",
              )}
            >
              <div className="flex items-start gap-2">
                <div className="flex min-w-0 flex-1 flex-col">
                  <h3 className="m-0 truncate text-body font-semibold text-fg-1">
                    {plan.title}
                  </h3>
                  {plan.subtitle ? (
                    <p className="m-0 text-caption text-fg-2">
                      {plan.subtitle}
                    </p>
                  ) : null}
                </div>
                {current ? (
                  <span className="shrink-0 rounded-full bg-action-soft px-2 text-caption font-semibold text-action-text">
                    {t("aicli.wallet.plans.current")}
                  </span>
                ) : null}
              </div>
              <div className="flex items-baseline gap-1.5">
                <span className="text-metric tabular-nums text-fg-1">
                  {fmt(planPriceQuota(plan, display))}
                </span>
                <span className="text-caption text-fg-2">
                  / {planDuration(plan, t)}
                </span>
              </div>
              <ul className="m-0 flex list-none flex-col gap-1 p-0 text-caption text-fg-2">
                {features.map((feature) => (
                  <li key={feature} className="flex items-start gap-1.5">
                    <Check
                      aria-hidden="true"
                      strokeWidth={2}
                      className="mt-0.5 h-3.5 w-3.5 shrink-0 text-action-text"
                    />
                    <span>{feature}</span>
                  </li>
                ))}
              </ul>
              <Button
                type="button"
                variant={current ? "solid" : "neutral"}
                size="regular"
                className="mt-auto w-full"
                disabled={buy.isPending}
                onClick={() => setChoosing(plan)}
              >
                {current
                  ? t("aicli.wallet.plans.renew")
                  : t("aicli.wallet.plans.subscribe")}
              </Button>
            </article>
          );
        })}
      </div>
      <p className="m-0 text-caption text-fg-3">
        {t("aicli.wallet.plans.chooseHint")}
      </p>

      <Dialog
        open={choosing != null}
        onOpenChange={(open) => {
          if (!open && !buy.isPending) setChoosing(null);
        }}
      >
        <DialogContent data-testid="aicli-wallet-plan-dialog">
          <DialogHeader>
            <DialogTitle>
              {t("aicli.wallet.plans.chooseTitle", {
                title: choosing?.title ?? "",
              })}
            </DialogTitle>
            <DialogDescription>
              {choosing
                ? t("aicli.wallet.plans.chooseBody", {
                    price: fmt(planPriceQuota(choosing, display)),
                    duration: planDuration(choosing, t),
                  })
                : ""}
            </DialogDescription>
          </DialogHeader>
          <div className="flex flex-col gap-2 px-6 pb-2">
            {methods.map((entry) => (
              <button
                key={entry.type}
                type="button"
                disabled={busy || buy.isPending}
                onClick={() => choosing && purchase(choosing, entry.type)}
                className={cn(
                  choiceClass(false),
                  "flex h-11 items-center gap-2.5 px-4 text-start text-body font-medium",
                )}
              >
                <MethodMark method={entry} />
                {entry.name}
              </button>
            ))}
            {choosing && choosing.allow_balance_pay !== false ? (
              <button
                type="button"
                disabled={buy.isPending}
                onClick={() => purchase(choosing, BALANCE_METHOD)}
                className={cn(
                  choiceClass(false),
                  "flex h-11 items-center gap-2.5 px-4 text-start text-body font-medium",
                )}
              >
                <Wallet
                  aria-hidden="true"
                  strokeWidth={1.5}
                  className="h-[22px] w-[22px] shrink-0 text-fg-2"
                />
                {t("aicli.wallet.plans.balanceOption", {
                  price: fmt(planPriceQuota(choosing, display)),
                })}
              </button>
            ) : null}
          </div>
          <DialogFooter>
            <Button
              type="button"
              variant="neutral"
              size="regular"
              disabled={buy.isPending}
              onClick={() => setChoosing(null)}
            >
              {t("common.cancel")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}

function InvitePanel({
  code,
  accountValue,
  display,
  locale,
  wide,
}: {
  code: string;
  accountValue: unknown;
  display: Display;
  locale: string;
  /** 独占一行时链接与统计左右排。 */
  wide: boolean;
}) {
  const { t } = useTranslation();
  const transfer = useAicliAffTransfer();
  const [confirming, setConfirming] = useState(false);
  const stats = readAffStats(accountValue);
  const link = inviteLink(code);
  const minimum = minAffTransfer(display);
  const fmt = (quota: number | null) =>
    formatAicliQuota(quota, display, locale) ?? t("aicli.account.noData");
  const canTransfer = stats.pending != null && stats.pending >= minimum;

  const copy = async () => {
    try {
      await copyText(link);
      toast.success(t("aicli.wallet.invite.copied"));
    } catch {
      toast.error(t("aicli.wallet.errors.generic"));
    }
  };

  return (
    <section
      aria-labelledby="aicli-wallet-invite-title"
      data-testid="aicli-wallet-invite"
      className={cn(
        panelClass,
        "flex-[2_1_480px]",
        wide && "flex-row flex-wrap items-end gap-x-7",
      )}
    >
      <div className="flex min-w-0 flex-[1_1_360px] flex-col gap-2.5">
        <PanelTitle id="aicli-wallet-invite-title">
          {t("aicli.wallet.invite.title")}
        </PanelTitle>
        <div className="flex gap-2">
          <Input
            readOnly
            aria-label={t("aicli.wallet.invite.link")}
            className="h-9 min-w-0 flex-1 bg-sidebar font-mono"
            value={link}
            onFocus={(event) => event.target.select()}
          />
          <Button
            type="button"
            variant="neutral"
            size="regular"
            className="h-9 gap-1.5"
            onClick={() => void copy()}
          >
            <Copy aria-hidden="true" className="h-4 w-4" strokeWidth={1.5} />
            {t("common.copy")}
          </Button>
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-x-7 gap-y-3">
        <Stat
          label={t("aicli.wallet.invite.pending")}
          value={fmt(stats.pending)}
        />
        <Stat label={t("aicli.wallet.invite.total")} value={fmt(stats.total)} />
        <Stat
          label={t("aicli.wallet.invite.invited")}
          value={
            stats.invited != null
              ? stats.invited.toLocaleString(locale)
              : t("aicli.account.noData")
          }
        />
        <div className="ms-auto flex flex-col items-end gap-1">
          <Button
            type="button"
            variant="neutral"
            size="regular"
            disabled={!canTransfer || transfer.isPending}
            onClick={() => setConfirming(true)}
          >
            {t("aicli.wallet.invite.transfer")}
          </Button>
          {!canTransfer ? (
            <span className="text-caption text-fg-3">
              {t("aicli.wallet.invite.transferHint", { min: fmt(minimum) })}
            </span>
          ) : null}
        </div>
      </div>
      <ConfirmDialog
        isOpen={confirming}
        variant="info"
        title={t("aicli.wallet.invite.confirmTitle", {
          value: fmt(stats.pending),
        })}
        message={t("aicli.wallet.invite.confirmMessage")}
        confirmText={t("aicli.wallet.invite.transfer")}
        pending={transfer.isPending}
        onConfirm={() => {
          if (stats.pending == null) return;
          transfer.mutate(stats.pending, {
            onSuccess: () =>
              toast.success(t("aicli.wallet.invite.transferred")),
            onError: (error) => toast.error(walletErrorText(error, t)),
            onSettled: () => setConfirming(false),
          });
        }}
        onCancel={() => setConfirming(false)}
      />
    </section>
  );
}

function HistoryPanel({
  methods,
  display,
  locale,
}: {
  methods: AicliPayMethod[];
  display: Display;
  locale: string;
}) {
  const { t } = useTranslation();
  const [page, setPage] = useState(1);
  const history = useAicliWalletHistory(page);
  const items = history.data?.items ?? [];
  const pages = Math.max(
    1,
    Math.ceil((history.data?.total ?? 0) / HISTORY_PAGE_SIZE),
  );
  const methodName = (type: string) =>
    methods.find((entry) => entry.type === type)?.name ?? type;
  const cell = "px-3 py-2.5 text-start align-middle";

  return (
    <section
      aria-labelledby="aicli-wallet-history-title"
      data-testid="aicli-wallet-history"
      className={cn(panelClass, "gap-3")}
    >
      <PanelTitle id="aicli-wallet-history-title">
        {t("aicli.wallet.history.title")}
      </PanelTitle>
      {history.isError && !history.data ? (
        <Notice
          tone="warning"
          title={walletErrorText(history.error, t)}
          actions={
            <Button
              type="button"
              variant="neutral"
              size="compact"
              onClick={() => void history.refetch()}
            >
              {t("common.refresh")}
            </Button>
          }
        />
      ) : items.length === 0 ? (
        <p className="m-0 text-body text-fg-3">
          {history.isPending
            ? t("common.loading")
            : t("aicli.wallet.history.empty")}
        </p>
      ) : (
        <div className="overflow-x-auto rounded-control border border-border">
          <table className="w-full min-w-[640px] border-collapse text-body tabular-nums">
            <thead className="bg-sidebar text-caption text-fg-2">
              <tr>
                {[
                  "time",
                  "amount",
                  "money",
                  "method",
                  "statusLabel",
                  "tradeNo",
                ].map((column) => (
                  <th
                    key={column}
                    scope="col"
                    className={cn(cell, "py-2 font-medium")}
                  >
                    {t(`aicli.wallet.history.${column}`)}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {items.map((order) => (
                <tr key={order.id} className="border-t border-border">
                  <td className={cn(cell, "whitespace-nowrap text-fg-2")}>
                    {order.create_time > 0
                      ? new Date(order.create_time * 1000).toLocaleString(
                          locale,
                        )
                      : ""}
                  </td>
                  <td className={cn(cell, "text-fg-1")}>
                    {formatAicliQuota(
                      order.amount * quotaPerUnit(display),
                      display,
                      locale,
                    )}
                  </td>
                  <td className={cn(cell, "text-fg-1")}>
                    {t("aicli.wallet.topup.money", {
                      money: order.money.toFixed(2),
                    })}
                  </td>
                  <td className={cn(cell, "text-fg-2")}>
                    {methodName(order.payment_method)}
                  </td>
                  <td className={cell}>
                    <span
                      className={cn(
                        "rounded-full px-2 py-0.5 text-caption font-semibold",
                        STATUS_TONE[order.status] ?? "bg-subtle text-fg-2",
                      )}
                    >
                      {["success", "pending", "expired", "failed"].includes(
                        order.status,
                      )
                        ? t(`aicli.wallet.history.status.${order.status}`)
                        : order.status}
                    </span>
                  </td>
                  <td className={cn(cell, "font-mono text-caption text-fg-3")}>
                    {order.trade_no}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {pages > 1 ? (
        <div className="flex items-center justify-end gap-2 text-caption text-fg-2">
          <span>{t("aicli.wallet.history.page", { page, pages })}</span>
          <Button
            type="button"
            variant="neutral"
            size="compact"
            disabled={page <= 1}
            onClick={() => setPage((current) => Math.max(1, current - 1))}
          >
            {t("aicli.wallet.history.prev")}
          </Button>
          <Button
            type="button"
            variant="neutral"
            size="compact"
            disabled={page >= pages}
            onClick={() => setPage((current) => Math.min(pages, current + 1))}
          >
            {t("aicli.wallet.history.next")}
          </Button>
        </div>
      ) : null}
    </section>
  );
}

/**
 * 钱包页（W 阶段，按 2026-11-15 定稿的设计稿）：上排左栏余额与「我的订阅」（没有生效的
 * 订阅时换成兑换码）、右栏在线充值；有套餐时下面一行订阅套餐；再下面兑换码与邀请返利、
 * 充值记录。全部走 new-api rc.34 现有接口；管理员没配置套餐时套餐与「我的订阅」都不出现。
 * 实付金额全部来自服务端；付款在独立的支付窗口里完成，到账以服务端为准，这里只轮询确认。
 */
export function AicliWalletPage() {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;
  const account = useAicliAccount().data;
  const bound = account?.state === "bound" || account?.state === "needs_rebind";
  const cached = useAicliCachedValue("account", bound);
  const wallet = useAicliWallet(bound);
  const { watch, start, dismiss } = useAicliPayWatch();
  const display = account?.quota_display;
  const view = wallet.data;
  const quota = readAccountQuota(cached.data?.value);
  const busy = watch.phase === "waiting";
  const methods = useMemo(() => view?.topup?.methods ?? [], [view]);
  const tokenInvalid = aicliErrorCode(wallet.error) === "token_invalid";
  const active = (view?.subscriptions ?? []).filter(
    (entry) => entry.status === "active",
  );
  const hasMine = active.length > 0;
  const redemption = Boolean(view?.topup?.redemption);
  const fetchedAt = cached.data?.fetched_at;

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <AppPageHeader
        icon={<Wallet className="h-5 w-5" strokeWidth={1.5} />}
        title={t("nav.wallet")}
        actions={
          <span className="text-caption text-fg-3">
            {t("aicli.account.source")}
            {fetchedAt
              ? ` · ${t("aicli.account.lastSync", {
                  time: formatSyncTime(fetchedAt, locale),
                })}`
              : ""}
          </span>
        }
      />
      <div
        id="main-content"
        data-testid="aicli-wallet"
        className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto scroll-stable px-6 pb-10 pt-4"
      >
        <PayStatus watch={watch} onDismiss={dismiss} />

        {wallet.isError && !view ? (
          <Notice
            tone={tokenInvalid ? "danger" : "warning"}
            title={
              tokenInvalid
                ? t("aicli.account.tokenInvalid")
                : t("aicli.wallet.loadFailed")
            }
            actions={
              <Button
                type="button"
                variant="neutral"
                size="compact"
                onClick={() => void wallet.refetch()}
              >
                {t("common.refresh")}
              </Button>
            }
          />
        ) : null}
        {view && view.failed.length > 0 ? (
          <Notice
            tone="warning"
            title={t("aicli.wallet.partFailed")}
            actions={
              <Button
                type="button"
                variant="neutral"
                size="compact"
                onClick={() => void wallet.refetch()}
              >
                {t("common.refresh")}
              </Button>
            }
          />
        ) : null}

        <div className="flex flex-wrap items-stretch gap-4">
          <div className="flex min-w-0 flex-[1_1_280px] flex-col gap-4">
            <BalanceCard
              remaining={quota.remaining}
              used={quota.used}
              display={display}
              locale={locale}
            />
            {view && hasMine ? (
              <MySubscriptions
                active={active}
                plans={view.plans}
                display={display}
                locale={locale}
              />
            ) : null}
            {view && !hasMine && redemption ? (
              <RedeemPanel display={display} locale={locale} stacked />
            ) : null}
          </div>
          {view?.topup ? (
            <div className="flex min-w-0 flex-[2_1_480px] flex-col">
              {view.topup.online_topup ? (
                <TopupPanel
                  info={view.topup}
                  display={display}
                  locale={locale}
                  busy={busy}
                  onStart={start}
                />
              ) : (
                <TopupOffPanel link={view.topup.topup_link} />
              )}
            </div>
          ) : null}
        </div>

        {view && view.plans.length > 0 ? (
          <PlansPanel
            plans={view.plans}
            subscriptions={view.subscriptions}
            methods={methods}
            display={display}
            locale={locale}
            busy={busy}
            onStart={start}
          />
        ) : null}

        {view && ((hasMine && redemption) || view.aff_code) ? (
          <div className="flex flex-wrap items-stretch gap-4">
            {hasMine && redemption ? (
              <div className="flex min-w-0 flex-[1_1_280px] flex-col">
                <RedeemPanel
                  display={display}
                  locale={locale}
                  stacked={false}
                />
              </div>
            ) : null}
            {view.aff_code ? (
              <InvitePanel
                code={view.aff_code}
                accountValue={cached.data?.value}
                display={display}
                locale={locale}
                wide={!(hasMine && redemption)}
              />
            ) : null}
          </div>
        ) : null}

        {view ? (
          <HistoryPanel methods={methods} display={display} locale={locale} />
        ) : null}
      </div>
    </div>
  );
}
