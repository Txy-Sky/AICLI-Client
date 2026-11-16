import React, { useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ExternalLink, Eye, EyeOff } from "lucide-react";
import { DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { HoverTip } from "@/components/ui/hover-tip";
import { Notice } from "@/components/ui/notice";
import { AppGlyph, APP_DISPLAY_NAME } from "@/components/shell/AppGlyph";
import { DiscoverLogo } from "@/components/discover/DiscoverLogo";
import type { McpAppId } from "@/config/appConfig";
import {
  applyInputs,
  firstMissingInput,
  type McpCatalogEntry,
  type McpCatalogInput,
} from "@/lib/mcpRegistry";
import { settingsApi } from "@/lib/api";
import { cn } from "@/lib/utils";
import {
  CHECKBOX_CLASS,
  FIELD_CLASS,
  LABEL_CLASS,
  MONO_FIELD_CLASS,
  V7Dialog,
} from "./formBits";
import { SSE_AS_HTTP_APPS, SSE_REJECTED_APPS, transportOf } from "./mcpDraft";
import {
  defaultDiscoverApps,
  discoverDescription,
  inputLabel,
  type DiscoverApps,
} from "./mcpDiscoverEntry";
import { DiscoverMeta } from "./McpDiscoverMeta";

/**
 * 发现栏「添加…」对话框（参考 magpie 的 serverSheet）：
 * 头部图标、名称、发布者 / 类型、主页；描述；要填的密钥 / 参数；写入到哪些应用。
 */
export function McpDiscoverAddDialog({
  entry,
  id,
  visibleAppIds,
  pending,
  onCancel,
  onConfirm,
}: {
  entry: McpCatalogEntry;
  /** 实际写入的键名（重名时已加后缀） */
  id: string;
  visibleAppIds: McpAppId[];
  pending: boolean;
  onCancel: () => void;
  onConfirm: (spec: McpCatalogEntry["spec"], apps: DiscoverApps) => void;
}) {
  const { t } = useTranslation();
  const [values, setValues] = useState<Record<string, string>>({});
  const [revealed, setRevealed] = useState<Record<string, boolean>>({});
  const [apps, setApps] = useState<DiscoverApps>(() =>
    defaultDiscoverApps(visibleAppIds, entry.spec),
  );
  const [missing, setMissing] = useState<McpCatalogInput | null>(null);
  const formRef = useRef<HTMLFormElement>(null);

  const description = discoverDescription(entry, t);
  const isSse = transportOf(entry.spec) === "sse";
  const appNames = (list: readonly McpAppId[]) =>
    list
      .filter((app) => apps[app])
      .map((app) => APP_DISPLAY_NAME[app])
      .join(t("mcpPage.listSeparator"));
  const sseAsHttp = isSse ? appNames(SSE_AS_HTTP_APPS) : "";
  const sseRejected = isSse ? appNames(SSE_REJECTED_APPS) : "";
  const noApps = useMemo(
    () => !visibleAppIds.some((app) => apps[app]),
    [apps, visibleAppIds],
  );

  const fieldId = (input: McpCatalogInput) => `mcp-discover-in-${input.key}`;

  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    if (pending) return;
    const lack = firstMissingInput(entry, values);
    if (lack) {
      setMissing(lack);
      document.getElementById(fieldId(lack))?.focus();
      return;
    }
    setMissing(null);
    onConfirm(applyInputs(entry, values), apps);
  };

  const hintOf = (input: McpCatalogInput) => {
    const parts: React.ReactNode[] = [];
    if (input.optionalNote === "rateLimit") {
      parts.push(t("mcpPage.discover.rateLimit"));
    }
    if (input.description) parts.push(input.description);
    if (input.obtain) {
      const url = `https://${input.obtain}`;
      parts.push(
        <span key="obtain">
          {t("mcpPage.discover.obtainBefore")}
          <button
            type="button"
            className="text-fg-1 underline decoration-border-strong underline-offset-2 hover:decoration-fg-2"
            onClick={() => void settingsApi.openExternal(url)}
          >
            {input.obtain}
          </button>
          {t("mcpPage.discover.obtainAfter")}
        </span>,
      );
    }
    parts.push(
      input.where === "env"
        ? t("mcpPage.discover.asEnv", { key: input.key })
        : input.where === "header"
          ? t("mcpPage.discover.asHeader", { key: input.key })
          : t("mcpPage.discover.asArg"),
    );
    return parts.flatMap((part, index) =>
      index === 0 ? [part] : [<span key={`sep-${index}`}> · </span>, part],
    );
  };

  return (
    <V7Dialog
      open
      width={600}
      onOpenChange={(open) => {
        if (!open && !pending) onCancel();
      }}
    >
      <form
        ref={formRef}
        onSubmit={submit}
        className="flex min-h-0 flex-col gap-4 overflow-y-auto"
      >
        <div className="flex items-center gap-3 border-b border-border pb-4">
          <DiscoverLogo src={entry.icon} name={entry.title} size={44} />
          <div className="flex min-w-0 flex-1 flex-col gap-1">
            <DialogTitle className="truncate text-title">
              {entry.title}
            </DialogTitle>
            <div className="flex min-w-0 items-center gap-1.5 text-caption text-fg-2">
              <DiscoverMeta entry={entry} />
            </div>
          </div>
          {entry.homepage && (
            <Button
              type="button"
              variant="quiet"
              size="compact"
              onClick={() => void settingsApi.openExternal(entry.homepage!)}
            >
              {t("mcpPage.discover.homepage")}
              <ExternalLink className="h-3.5 w-3.5" strokeWidth={2} />
            </Button>
          )}
        </div>

        <DialogDescription className="m-0 text-body text-fg-1">
          {description || t("mcpPage.discover.noDescription")}
        </DialogDescription>
        {entry.signIn && (
          <p className="m-0 text-caption text-fg-2">
            {t("mcpPage.discover.signInNote", { name: entry.title })}
          </p>
        )}

        {entry.inputs.map((input) => {
          const label = inputLabel(input, t);
          const isMissing = missing?.key === input.key;
          const shown = revealed[input.key];
          return (
            <div key={input.key} className="flex flex-col gap-1.5">
              <label htmlFor={fieldId(input)} className={LABEL_CLASS}>
                {label}
                {!input.required && (
                  <span className="font-normal text-fg-2">
                    {t("mcpPage.discover.optional")}
                  </span>
                )}
              </label>
              <div className="relative">
                <input
                  id={fieldId(input)}
                  type={input.secret && !shown ? "password" : "text"}
                  autoComplete="off"
                  spellCheck={false}
                  placeholder={
                    input.placeholder ??
                    (input.where === "env" ? input.key : undefined)
                  }
                  aria-invalid={isMissing || undefined}
                  aria-describedby={`${fieldId(input)}-hint`}
                  className={cn(
                    input.secret ? MONO_FIELD_CLASS : FIELD_CLASS,
                    input.secret && "pe-9",
                    isMissing && "border-danger",
                  )}
                  value={values[input.key] ?? ""}
                  onChange={(event) => {
                    const value = event.target.value;
                    setValues((prev) => ({ ...prev, [input.key]: value }));
                    if (isMissing) setMissing(null);
                  }}
                />
                {input.secret && (
                  <HoverTip
                    content={
                      shown
                        ? t("mcpPage.discover.hideValue")
                        : t("mcpPage.discover.showValue")
                    }
                  >
                    <button
                      type="button"
                      aria-label={
                        shown
                          ? t("mcpPage.discover.hideValue")
                          : t("mcpPage.discover.showValue")
                      }
                      onClick={() =>
                        setRevealed((prev) => ({
                          ...prev,
                          [input.key]: !prev[input.key],
                        }))
                      }
                      className="absolute end-1 top-1/2 inline-flex h-7 w-7 -translate-y-1/2 items-center justify-center rounded-control text-fg-2 hover:bg-subtle hover:text-fg-1"
                    >
                      {shown ? (
                        <EyeOff className="h-3.5 w-3.5" strokeWidth={2} />
                      ) : (
                        <Eye className="h-3.5 w-3.5" strokeWidth={2} />
                      )}
                    </button>
                  </HoverTip>
                )}
              </div>
              <p
                id={`${fieldId(input)}-hint`}
                className={cn(
                  "m-0 text-caption [overflow-wrap:anywhere]",
                  isMissing ? "text-danger-text" : "text-fg-2",
                )}
              >
                {isMissing
                  ? t("mcpPage.discover.fillRequired", { label })
                  : hintOf(input)}
              </p>
            </div>
          );
        })}

        <div
          role="group"
          aria-labelledby="mcp-discover-apps"
          className="flex flex-col gap-2"
        >
          <span id="mcp-discover-apps" className={LABEL_CLASS}>
            {t("mcpPage.drawer.writeTo")}
          </span>
          <div className="grid grid-cols-2 gap-x-4 gap-y-2 sm:grid-cols-3">
            {visibleAppIds.map((app) => (
              <label
                key={app}
                className="flex h-6 cursor-pointer items-center gap-2 text-body"
              >
                <input
                  type="checkbox"
                  className={CHECKBOX_CLASS}
                  checked={apps[app]}
                  onChange={(event) =>
                    setApps((prev) => ({
                      ...prev,
                      [app]: event.target.checked,
                    }))
                  }
                />
                <AppGlyph app={app} size={16} badgeClassName="bg-surface" />
                <span className="truncate">{APP_DISPLAY_NAME[app]}</span>
              </label>
            ))}
          </div>
          {sseAsHttp && (
            <Notice
              tone="warning"
              title={t("mcpPage.drawer.sseAsHttpTitle", { apps: sseAsHttp })}
            >
              {t("mcpPage.drawer.sseAsHttpBody", { apps: sseAsHttp })}
            </Notice>
          )}
          {sseRejected && (
            <Notice
              tone="warning"
              title={t("mcpPage.drawer.sseRejectedTitle", {
                apps: sseRejected,
              })}
            >
              {t("mcpPage.drawer.sseRejectedBody")}
            </Notice>
          )}
        </div>

        <div className="flex items-center gap-2 border-t border-border pt-4">
          <span
            className="min-w-0 flex-1 truncate font-mono text-caption text-fg-3"
            title={id}
          >
            {id}
          </span>
          <Button
            type="button"
            variant="neutral"
            size="regular"
            disabled={pending}
            onClick={onCancel}
          >
            {t("common.cancel")}
          </Button>
          <Button
            type="submit"
            variant="solid"
            size="regular"
            disabled={pending}
          >
            {pending
              ? t("mcpPage.discover.adding")
              : noApps
                ? t("mcpPage.discover.addOnly")
                : t("mcpPage.discover.add")}
          </Button>
        </div>
      </form>
    </V7Dialog>
  );
}
