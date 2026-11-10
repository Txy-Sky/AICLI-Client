import { useEffect, useRef, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, ExternalLink, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { settingsApi } from "@/lib/api/settings";
import { aicliErrorCode, isAicliCommandError } from "@/lib/api/aicli";
import { useAicliBind } from "@/hooks/useAicliAccount";
import type { AicliBindResult } from "@/types/aicli";

/** 网页「个人设置」地址（清单 0.4 节固定取值）。 */
export const AICLI_PROFILE_URL = "https://aicliapi.com/profile";

interface AicliBindCardProps {
  /** 绑定使用的线路（默认 main）。 */
  line?: string;
  /** 变化时把焦点放到输入框（深链 aicli-open-bind）。 */
  focusSignal?: number;
  onBound?: (result: AicliBindResult) => void;
  className?: string;
}

/**
 * 绑定界面（F-03）：标题、占位、说明、警告按 0.4 节文案常驻显示；密码型输入框，
 * 提交后立刻清空（令牌不留在界面状态里）。粘贴 sk- 开头的值时直接提示，不发起绑定。
 *
 * 单列排布：标题行（右侧是去网页拿令牌的链接）→ 整行输入框 → 说明 → 整行绑定按钮 →
 * 一行小字警告。主操作只有「绑定」一个，拿令牌是次要操作，所以做成链接样式。
 */
export function AicliBindCard({
  line = "main",
  focusSignal,
  onBound,
  className,
}: AicliBindCardProps) {
  const { t } = useTranslation();
  const bind = useAicliBind();
  const inputRef = useRef<HTMLInputElement>(null);
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (focusSignal) inputRef.current?.focus();
  }, [focusSignal]);

  const submit = (event: FormEvent) => {
    event.preventDefault();
    const token = value.trim();
    setValue("");
    if (!token) return;
    if (token.startsWith("sk-")) {
      setError(t("aicli.bind.skKeyError"));
      return;
    }
    setError(null);
    bind.mutate(
      { accessToken: token, line },
      {
        onSuccess: (result) => onBound?.(result),
        onError: (failure) => {
          if (aicliErrorCode(failure) === "wrong_credential_type") {
            setError(t("aicli.bind.skKeyError"));
            return;
          }
          const message = isAicliCommandError(failure)
            ? failure.message
            : String(failure);
          setError(t("aicli.bind.failed", { message }));
        },
      },
    );
  };

  return (
    <section
      aria-labelledby="aicli-bind-title"
      className={
        className ??
        "flex flex-col gap-3 rounded-panel border border-border bg-surface p-4"
      }
    >
      <form className="flex flex-col gap-2" onSubmit={submit}>
        <div className="flex items-center justify-between gap-3">
          <h2
            id="aicli-bind-title"
            className="m-0 text-body font-medium text-fg-1"
          >
            {t("aicli.bind.title")}
          </h2>
          <Button
            type="button"
            variant="link"
            size="compact"
            className="h-auto gap-1 px-0"
            onClick={() => void settingsApi.openExternal(AICLI_PROFILE_URL)}
          >
            {t("aicli.bind.openWeb")}
            <ExternalLink aria-hidden="true" className="h-3.5 w-3.5" />
          </Button>
        </div>
        <Input
          ref={inputRef}
          type="password"
          name="aicli-access-token"
          autoComplete="off"
          aria-label={t("aicli.bind.title")}
          aria-invalid={error ? true : undefined}
          aria-describedby={
            error
              ? "aicli-bind-error aicli-bind-description"
              : "aicli-bind-description"
          }
          placeholder={t("aicli.bind.placeholder")}
          value={value}
          onChange={(event) => {
            setValue(event.target.value);
            if (error) setError(null);
          }}
          disabled={bind.isPending}
          className="h-9 w-full"
        />
        {error && (
          <p
            id="aicli-bind-error"
            role="alert"
            className="m-0 text-caption text-danger-text"
          >
            {error}
          </p>
        )}
        <p id="aicli-bind-description" className="m-0 text-caption text-fg-2">
          {t("aicli.bind.description")}
        </p>
        <Button
          type="submit"
          variant="solid"
          size="regular"
          className="mt-2 h-9 w-full"
          disabled={bind.isPending || value.trim() === ""}
        >
          {bind.isPending && (
            <Loader2 aria-hidden="true" className="h-3.5 w-3.5 animate-spin" />
          )}
          {bind.isPending ? t("aicli.bind.binding") : t("aicli.bind.submit")}
        </Button>
      </form>
      <p className="m-0 flex items-start gap-1.5 text-caption text-fg-2">
        <AlertTriangle
          aria-hidden="true"
          strokeWidth={1.5}
          className="mt-[2px] h-3.5 w-3.5 shrink-0 text-warning-text"
        />
        <span>{t("aicli.bind.warning")}</span>
      </p>
    </section>
  );
}
