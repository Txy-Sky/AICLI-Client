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
          <DialogContent
            zIndex="top"
            className="max-w-md gap-6 p-7"
            onEscapeKeyDown={(event) => event.preventDefault()}
          >
            <DialogHeader className="items-center gap-1 p-0 text-center">
              <img
                src={appLogo}
                alt=""
                draggable={false}
                className="mb-2 h-12 w-12"
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
