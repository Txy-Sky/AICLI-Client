import { useEffect, useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { AlertTriangle, Check, Info } from "lucide-react";
import { useTranslation } from "react-i18next";

interface ConfirmDialogProps {
  isOpen: boolean;
  title: string;
  /** 一句结论：会发生什么 */
  message: string;
  /** 灰底补充：不受影响、可以放心的事（2026-11 改版，设计稿画板 23） */
  assurance?: string;
  /** 橙底补充：要注意的风险 */
  note?: string;
  /** 一段要运行的命令，等宽字体显示 */
  code?: string;
  /** 正文最后的小字说明 */
  hint?: string;
  confirmText?: string;
  cancelText?: string;
  variant?: "destructive" | "info";
  zIndex?: "base" | "nested" | "alert" | "top";
  /** 可选勾选项：提供 label 即显示，勾选状态经 onConfirm 参数回传 */
  checkboxLabel?: string;
  /** 勾选项下面的小字说明 */
  checkboxHint?: string;
  checkboxDefaultChecked?: boolean;
  pending?: boolean;
  onConfirm: (checkboxChecked: boolean) => void;
  onCancel: () => void;
}

export function ConfirmDialog({
  isOpen,
  title,
  message,
  assurance,
  note,
  code,
  hint,
  confirmText,
  cancelText,
  variant = "destructive",
  zIndex = "alert",
  checkboxLabel,
  checkboxHint,
  checkboxDefaultChecked = false,
  pending = false,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const { t } = useTranslation();
  const [checkboxChecked, setCheckboxChecked] = useState(
    checkboxDefaultChecked,
  );

  useEffect(() => {
    if (isOpen) {
      setCheckboxChecked(checkboxDefaultChecked);
    }
  }, [isOpen, checkboxDefaultChecked]);

  const IconComponent = variant === "info" ? Info : AlertTriangle;
  const iconClass =
    variant === "info" ? "h-5 w-5 text-fg-1" : "h-5 w-5 text-destructive";

  return (
    <Dialog
      open={isOpen}
      onOpenChange={(open) => {
        if (!open && !pending) {
          onCancel();
        }
      }}
    >
      <DialogContent className="max-w-sm" zIndex={zIndex}>
        <DialogHeader className="space-y-3 border-b-0 bg-transparent pb-0">
          <DialogTitle className="flex items-center gap-2 text-lg font-semibold">
            <IconComponent className={iconClass} />
            {title}
          </DialogTitle>
        </DialogHeader>
        {/* 正文（2026-11 改版）：一句结论，必要时补一行放心的事 / 风险 / 命令，勾选项做成卡片。
            外框、标题行和按钮保持原样。 */}
        <div className="flex flex-col gap-3 px-6 pt-3">
          <DialogDescription className="whitespace-pre-line text-sm leading-relaxed">
            {message}
          </DialogDescription>
          {assurance ? (
            <p
              data-testid="confirm-assurance"
              className="m-0 flex items-start gap-2 rounded-control bg-subtle px-3 py-2 text-caption text-fg-2"
            >
              <Check
                aria-hidden="true"
                className="mt-0.5 h-3.5 w-3.5 shrink-0 text-action-text"
                strokeWidth={2.5}
              />
              <span>{assurance}</span>
            </p>
          ) : null}
          {note ? (
            <p
              data-testid="confirm-note"
              className="m-0 flex items-start gap-2 rounded-control bg-warning-soft px-3 py-2 text-caption text-warning-text"
            >
              <AlertTriangle
                aria-hidden="true"
                className="mt-0.5 h-3.5 w-3.5 shrink-0"
              />
              <span>{note}</span>
            </p>
          ) : null}
          {code ? (
            <code
              data-testid="confirm-code"
              className="block rounded-control bg-subtle px-3 py-2 font-mono text-caption text-fg-1 [overflow-wrap:anywhere]"
            >
              {code}
            </code>
          ) : null}
          {checkboxLabel ? (
            <label className="flex cursor-pointer select-none items-start gap-2.5 rounded-control border border-border px-3 py-2.5">
              <Checkbox
                checked={checkboxChecked}
                disabled={pending}
                onCheckedChange={(value) => setCheckboxChecked(value === true)}
                className="mt-0.5"
              />
              <span className="flex flex-col gap-0.5">
                <span className="text-sm leading-relaxed">{checkboxLabel}</span>
                {checkboxHint ? (
                  <span className="text-caption text-fg-3">{checkboxHint}</span>
                ) : null}
              </span>
            </label>
          ) : null}
          {hint ? <p className="m-0 text-caption text-fg-3">{hint}</p> : null}
        </div>
        <DialogFooter className="flex gap-2 border-t-0 bg-transparent pt-2 sm:justify-end">
          <Button variant="outline" onClick={onCancel} disabled={pending}>
            {cancelText || t("common.cancel")}
          </Button>
          <Button
            variant={variant === "info" ? "default" : "destructive"}
            disabled={pending}
            onClick={() =>
              // 未渲染勾选框时不得回传 defaultChecked 残留值
              onConfirm(checkboxLabel ? checkboxChecked : false)
            }
          >
            {confirmText || t("common.confirm")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
