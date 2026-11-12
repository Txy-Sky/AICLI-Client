import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";
import { Check, Cloud, SquareTerminal } from "lucide-react";
import appLogo from "@/assets/icons/aicli-logo.png";
import { Button } from "@/components/ui/button";
import { Notice } from "@/components/ui/notice";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { WindowControlsContext } from "@/components/shell/AppPageHeader";
import { useAicliAccount } from "@/hooks/useAicliAccount";
import { DRAG_REGION_ATTR, DRAG_REGION_STYLE } from "@/lib/platform";
import { isAicliCommandError } from "@/lib/api/aicli";
import { AicliBindCard } from "./AicliBindCard";

/** 主界面里「重新绑定」入口用：打开 Gate 的绑定对话框。 */
const BindGateContext = createContext<() => void>(() => undefined);

export function useOpenAicliBind(): () => void {
  return useContext(BindGateContext);
}

interface AicliBindGateProps {
  /** 深链 aicli-open-bind 的计数：每变一次打开绑定对话框。 */
  bindRequest?: number;
  children: ReactNode;
}

/**
 * 绑定拦截（F-07）：未绑定时不进入应用，只显示品牌背景和不可关闭的绑定对话框，
 * 绑定成功后进入；令牌失效（needs_rebind）时进入应用并弹出可关闭的对话框，
 * 缓存仍可查看（F-03）。读取账户出错时不拦截，免得把用户锁在外面。
 * 对话框内容就是 AicliBindCard，第 0.4 节文案逐字不变。
 */
export function AicliBindGate({ bindRequest, children }: AicliBindGateProps) {
  const { t } = useTranslation();
  const account = useAicliAccount();
  const state = account.data?.state;
  const line = account.data?.selected_line_id ?? "main";
  const [open, setOpen] = useState(false);
  const promptedRebind = useRef(false);

  // 令牌失效：本次运行只主动弹一次，用户点「稍后」后靠首页的按钮再打开
  useEffect(() => {
    if (state === "needs_rebind" && !promptedRebind.current) {
      promptedRebind.current = true;
      setOpen(true);
    }
    if (state === "bound") promptedRebind.current = false;
  }, [state]);

  useEffect(() => {
    if (bindRequest) setOpen(true);
  }, [bindRequest]);

  const openBind = useCallback(() => setOpen(true), []);

  if (account.isPending) {
    return <GateBackdrop testId="aicli-bind-gate-loading" />;
  }

  if (state === "not_bound") {
    return (
      <GateBackdrop testId="aicli-bind-gate">
        <Dialog open>
          {/* 两栏：左边说明这是什么（窄窗口下收起），右边是绑定表单。
              右栏里 AicliBindCard 的文案按清单 0.4 节逐字不变。 */}
          <DialogContent
            zIndex="top"
            className="max-w-md gap-0 overflow-hidden p-0 md:max-w-[840px] md:flex-row"
            onEscapeKeyDown={(event) => event.preventDefault()}
          >
            <BindIntro />
            <div className="flex min-w-0 flex-1 flex-col justify-center gap-6 p-7 md:p-9">
              <DialogHeader className="items-center gap-1 p-0 text-center md:items-start md:text-start">
                <img
                  src={appLogo}
                  alt=""
                  draggable={false}
                  className="mb-2 h-12 w-12 md:hidden"
                />
                <DialogTitle>{t("aicli.bind.gateTitle")}</DialogTitle>
                <DialogDescription>
                  {t("aicli.bind.gateSubtitle")}
                </DialogDescription>
              </DialogHeader>
              <AicliBindCard
                line={line}
                focusSignal={bindRequest}
                className="flex flex-col gap-3"
              />
            </div>
          </DialogContent>
        </Dialog>
      </GateBackdrop>
    );
  }

  const bound = state === "bound" || state === "needs_rebind";
  const loadError = account.error
    ? isAicliCommandError(account.error)
      ? account.error.message
      : String(account.error)
    : null;

  return (
    <BindGateContext.Provider value={openBind}>
      {loadError && !account.data && (
        <div className="fixed inset-x-0 bottom-4 z-[60] mx-auto w-fit max-w-[90vw]">
          <Notice
            tone="danger"
            title={t("aicli.bind.loadFailed", { message: loadError })}
          />
        </div>
      )}
      {children}
      {bound && (
        <Dialog open={open} onOpenChange={setOpen}>
          <DialogContent
            zIndex="top"
            className="max-w-md gap-5 p-7"
            {...(state === "needs_rebind"
              ? {}
              : { "aria-describedby": undefined })}
          >
            <DialogHeader className="gap-1 p-0">
              <DialogTitle>{t("aicli.bind.rebindTitle")}</DialogTitle>
              {state === "needs_rebind" && (
                <DialogDescription>
                  {t("aicli.account.tokenInvalid")}
                </DialogDescription>
              )}
            </DialogHeader>
            <AicliBindCard
              line={line}
              focusSignal={bindRequest}
              onBound={() => setOpen(false)}
              className="flex flex-col gap-3"
            />
            <DialogFooter className="p-0">
              <Button
                type="button"
                variant="quiet"
                size="regular"
                onClick={() => setOpen(false)}
              >
                {t("aicli.bind.later")}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      )}
    </BindGateContext.Provider>
  );
}

/**
 * 首次绑定对话框的左栏：一句话说明、三格示意图（你的 AI 工具 → 这台电脑 → AICLI 服务）、
 * 三条好处。纯说明，没有操作；窄窗口（md 以下）不显示，只留右边的表单。
 */
function BindIntro() {
  const { t } = useTranslation();
  const benefits = [
    t("aicli.bind.benefitTakeover"),
    t("aicli.bind.benefitPrivacy"),
    t("aicli.bind.benefitUsage"),
  ];
  return (
    <div className="hidden w-1/2 shrink-0 flex-col gap-7 bg-action-soft p-9 md:flex">
      <div className="flex items-center gap-2 text-strong font-semibold text-fg-1">
        <img src={appLogo} alt="" draggable={false} className="h-7 w-7" />
        AICLI Client
      </div>
      <div className="flex flex-col gap-2">
        <p className="m-0 text-metric font-bold text-fg-1">
          {t("aicli.bind.heroTitle")}
        </p>
        <p className="m-0 text-strong font-normal text-fg-2">
          {t("aicli.bind.heroSubtitle")}
        </p>
      </div>
      <div
        role="img"
        aria-label={t("aicli.bind.flowLabel")}
        className="flex items-start gap-1.5"
      >
        <FlowNode label={t("aicli.bind.flowTools")}>
          <SquareTerminal
            aria-hidden="true"
            strokeWidth={1.75}
            className="h-6 w-6 text-fg-1"
          />
        </FlowNode>
        <FlowLink />
        <FlowNode label={t("aicli.bind.flowDevice")}>
          <img src={appLogo} alt="" draggable={false} className="h-9 w-9" />
        </FlowNode>
        <FlowLink />
        <FlowNode label={t("aicli.bind.flowService")}>
          <Cloud
            aria-hidden="true"
            strokeWidth={1.75}
            className="h-6 w-6 text-action-text"
          />
        </FlowNode>
      </div>
      <ul className="m-0 mt-auto flex list-none flex-col gap-2.5 p-0 text-strong font-normal text-fg-1">
        {benefits.map((benefit) => (
          <li key={benefit} className="flex items-center gap-2">
            <Check
              aria-hidden="true"
              strokeWidth={2.25}
              className="h-4 w-4 shrink-0 text-action-text"
            />
            {benefit}
          </li>
        ))}
      </ul>
    </div>
  );
}

function FlowNode({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex w-[84px] shrink-0 flex-col items-center gap-2">
      <span className="flex h-14 w-14 items-center justify-center rounded-dialog border border-border bg-surface">
        {children}
      </span>
      <span className="text-center text-caption font-medium text-fg-1">
        {label}
      </span>
    </div>
  );
}

function FlowLink() {
  return (
    <span aria-hidden="true" className="flex h-14 min-w-0 flex-1 items-center">
      <span className="block w-full border-t-2 border-dashed border-action" />
    </span>
  );
}

/** 拦截时的整窗背景：可拖动的顶条（无边框窗口要能拖、能关）、居中品牌。 */
function GateBackdrop({
  testId,
  children,
}: {
  testId: string;
  children?: ReactNode;
}) {
  const windowControls = useContext(WindowControlsContext);
  const dragRegionAttr = windowControls
    ? { "data-tauri-drag-region": true }
    : DRAG_REGION_ATTR;
  return (
    <div
      data-testid={testId}
      className="flex h-screen flex-col overflow-hidden bg-app text-fg-1"
    >
      <div
        className="flex h-11 shrink-0 items-center justify-end pe-2"
        {...dragRegionAttr}
        style={DRAG_REGION_STYLE as React.CSSProperties}
      >
        {windowControls}
      </div>
      <div
        className="flex flex-1 flex-col items-center justify-center gap-3 pb-16"
        {...dragRegionAttr}
      >
        <img
          src={appLogo}
          alt=""
          draggable={false}
          className="pointer-events-none h-14 w-14"
        />
        <span className="pointer-events-none text-title text-fg-1">
          AICLI Client
        </span>
      </div>
      {children}
    </div>
  );
}
