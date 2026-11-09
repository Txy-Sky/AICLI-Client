import { useEffect, useRef, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { ExternalLink, KeyRound, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Notice } from "@/components/ui/notice";
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
      <div className="flex items-center gap-2">
        <KeyRound
          aria-hidden="true"
          className="h-4 w-4 text-fg-2"
          strokeWidth={1.5}
        />
        <h2 id="aicli-bind-title" className="m-0 text-section text-fg-1">
          {t("aicli.bind.title")}
        </h2>
      </div>
      <p className="m-0 text-body text-fg-2">{t("aicli.bind.description")}</p>
      <Notice tone="warning" title={t("aicli.bind.warning")} />
      <form className="flex flex-wrap items-center gap-2" onSubmit={submit}>
        <Input
          ref={inputRef}
          type="password"
          name="aicli-access-token"
          aria-label={t("aicli.bind.title")}
          aria-invalid={error ? true : undefined}
          aria-describedby={error ? "aicli-bind-error" : undefined}
          placeholder={t("aicli.bind.placeholder")}
          value={value}
          onChange={(event) => {
            setValue(event.target.value);
            if (error) setError(null);
          }}
          disabled={bind.isPending}
          className="min-w-[240px] flex-1"
        />
        <Button
          type="submit"
          variant="solid"
          size="regular"
          disabled={bind.isPending || value.trim() === ""}
        >
          {bind.isPending && (
            <Loader2 aria-hidden="true" className="h-3.5 w-3.5 animate-spin" />
          )}
          {bind.isPending ? t("aicli.bind.binding") : t("aicli.bind.submit")}
        </Button>
        <Button
          type="button"
          variant="neutral"
          size="regular"
          onClick={() => void settingsApi.openExternal(AICLI_PROFILE_URL)}
        >
          <ExternalLink aria-hidden="true" className="h-3.5 w-3.5" />
          {t("aicli.bind.openWeb")}
        </Button>
      </form>
      {error && (
        <p
          id="aicli-bind-error"
          role="alert"
          className="m-0 text-caption text-danger-text"
        >
          {error}
        </p>
      )}
    </section>
  );
}
