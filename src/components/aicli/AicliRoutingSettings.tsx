import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronRight, Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Notice } from "@/components/ui/notice";
import { Textarea } from "@/components/ui/textarea";
import {
  SettingsBlock,
  SettingsCard,
  SettingsRow,
  SettingsSwitchRow,
} from "@/components/settings/SettingsLayout";
import { APP_DISPLAY_NAME, AppGlyph } from "@/components/shell/AppGlyph";
import type { AppId } from "@/lib/api";
import { isAicliCommandError } from "@/lib/api/aicli";
import {
  useAicliPrivacy,
  useAicliSetPrivacy,
  useAicliSetPrivacyCustom,
} from "@/hooks/useAicliAccount";
import { useAicliLines } from "@/hooks/useAicliLines";
import { useAicliTakeoverStatus } from "@/hooks/useAicliProfiles";
import type { AicliPrivacyRule, AicliPrivacyView } from "@/types/aicli";

type RuleDraft = { kind: string; mode: "prefix" | "regex"; value: string };

function toDrafts(rules: AicliPrivacyRule[]): RuleDraft[] {
  return rules.map((rule) =>
    rule.prefix
      ? { kind: rule.kind, mode: "prefix", value: rule.prefix }
      : { kind: rule.kind, mode: "regex", value: rule.regex ?? "" },
  );
}

function fromDrafts(drafts: RuleDraft[]): AicliPrivacyRule[] {
  return drafts
    .filter((draft) => draft.value.trim() !== "")
    .map((draft) =>
      draft.mode === "prefix"
        ? { kind: draft.kind, prefix: draft.value }
        : { kind: draft.kind, regex: draft.value },
    );
}

/** 自定义遮盖词与规则编辑（整体保存，后端整理并校验）。 */
function CustomTerms({ privacy }: { privacy: AicliPrivacyView }) {
  const { t } = useTranslation();
  const save = useAicliSetPrivacyCustom();
  const [words, setWords] = useState(privacy.custom_words.join("\n"));
  const [rules, setRules] = useState<RuleDraft[]>(
    toDrafts(privacy.custom_rules),
  );
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(
    null,
  );

  useEffect(() => {
    setWords(privacy.custom_words.join("\n"));
    setRules(toDrafts(privacy.custom_rules));
  }, [privacy.custom_words, privacy.custom_rules]);

  const submit = () => {
    setMessage(null);
    save.mutate(
      {
        words: words
          .split("\n")
          .map((word) => word.trim())
          .filter(Boolean),
        rules: fromDrafts(rules),
      },
      {
        onSuccess: () =>
          setMessage({ ok: true, text: t("aicli.settings.customSaved") }),
        onError: (failure) =>
          setMessage({
            ok: false,
            text: t("aicli.settings.customInvalid", {
              message: isAicliCommandError(failure)
                ? failure.message
                : String(failure),
            }),
          }),
      },
    );
  };

  const update = (index: number, patch: Partial<RuleDraft>) =>
    setRules((current) =>
      current.map((rule, i) => (i === index ? { ...rule, ...patch } : rule)),
    );

  return (
    <div className="flex flex-col gap-3 px-4 py-3">
      <label className="flex flex-col gap-1 text-body">
        <span className="text-fg-1">{t("aicli.settings.customWords")}</span>
        <span className="text-caption text-fg-3">
          {t("aicli.settings.customWordsHint")}
        </span>
        <Textarea
          aria-label={t("aicli.settings.customWords")}
          rows={4}
          value={words}
          onChange={(event) => setWords(event.target.value)}
        />
      </label>
      <div className="flex flex-col gap-2">
        <span className="text-body text-fg-1">
          {t("aicli.settings.customRules")}
        </span>
        <span className="text-caption text-fg-3">
          {t("aicli.settings.customRulesHint")}
        </span>
        {rules.map((rule, index) => (
          <div
            key={index}
            data-testid={`aicli-rule-${index}`}
            className="flex flex-wrap items-center gap-2"
          >
            <Input
              aria-label={t("aicli.settings.ruleKind")}
              placeholder={t("aicli.settings.ruleKind")}
              value={rule.kind}
              onChange={(event) => update(index, { kind: event.target.value })}
              className="w-36"
            />
            <select
              aria-label={t("aicli.settings.ruleMode")}
              className="rounded-control border border-border-strong bg-surface px-1.5 py-1 text-body"
              value={rule.mode}
              onChange={(event) =>
                update(index, {
                  mode: event.target.value as RuleDraft["mode"],
                })
              }
            >
              <option value="prefix">{t("aicli.settings.rulePrefix")}</option>
              <option value="regex">{t("aicli.settings.ruleRegex")}</option>
            </select>
            <Input
              aria-label={t("aicli.settings.ruleValue")}
              placeholder={
                rule.mode === "prefix"
                  ? t("aicli.settings.rulePrefixPlaceholder")
                  : t("aicli.settings.ruleRegexPlaceholder")
              }
              value={rule.value}
              onChange={(event) => update(index, { value: event.target.value })}
              className="min-w-[180px] flex-1"
            />
            <Button
              type="button"
              variant="quiet"
              size="icon-compact"
              aria-label={t("aicli.settings.removeRule")}
              onClick={() =>
                setRules((current) => current.filter((_, i) => i !== index))
              }
            >
              <Trash2 aria-hidden="true" className="h-3.5 w-3.5" />
            </Button>
          </div>
        ))}
        <div>
          <Button
            type="button"
            variant="quiet"
            size="compact"
            onClick={() =>
              setRules((current) => [
                ...current,
                { kind: "", mode: "prefix", value: "" },
              ])
            }
          >
            <Plus aria-hidden="true" className="h-3.5 w-3.5" />
            {t("aicli.settings.addRule")}
          </Button>
        </div>
      </div>
      <div className="flex items-center gap-3">
        <Button
          type="button"
          variant="neutral"
          size="compact"
          disabled={save.isPending}
          onClick={submit}
        >
          {t("aicli.settings.saveCustom")}
        </Button>
        {message && (
          <span
            role={message.ok ? "status" : "alert"}
            className={
              message.ok
                ? "text-caption text-fg-2"
                : "text-caption text-danger-text"
            }
          >
            {message.text}
          </span>
        )}
      </div>
    </div>
  );
}

/**
 * 设置 → 本地路由里的 AICLI 部分（F-06）：线路说明、隐私（开关、失败模式、允许
 * 未扫描媒体、自定义词和规则）、凭据存储降级提示、接管中的客户端。
 */
export function AicliRoutingSettings({
  onOpenApp,
}: {
  onOpenApp: (app: AppId) => void;
}) {
  const { t } = useTranslation();
  const privacy = useAicliPrivacy().data;
  const setPrivacy = useAicliSetPrivacy();
  const lines = useAicliLines().data;
  const takeovers = useAicliTakeoverStatus().data ?? [];
  const active = takeovers.filter((entry) => entry.state !== "released");

  const updatePrivacy = (patch: {
    privacyEnabled?: boolean;
    privacyFailureMode?: "block" | "pass";
    allowUnscannedMedia?: boolean;
  }) => {
    if (!privacy) return;
    setPrivacy.mutate({
      privacyEnabled: patch.privacyEnabled ?? privacy.privacy_enabled,
      privacyFailureMode:
        patch.privacyFailureMode ?? privacy.privacy_failure_mode,
      allowUnscannedMedia:
        patch.allowUnscannedMedia ?? privacy.allow_unscanned_media,
    });
  };

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
        {privacy && !privacy.bound && (
          <Notice tone="neutral" title={t("aicli.client.notBound")} />
        )}
        {privacy && (
          <SettingsCard>
            <SettingsSwitchRow
              label={t("aicli.settings.privacyEnabled")}
              description={t("aicli.settings.privacyEnabledHint")}
              checked={privacy.privacy_enabled}
              disabled={!privacy.bound || setPrivacy.isPending}
              onCheckedChange={(value) =>
                updatePrivacy({ privacyEnabled: value })
              }
            />
            <SettingsRow
              label={t("aicli.settings.failureMode")}
              description={
                privacy.privacy_failure_mode === "pass"
                  ? t("aicli.settings.failureModePassHint")
                  : t("aicli.settings.failureModeBlockHint")
              }
              control={
                <select
                  aria-label={t("aicli.settings.failureMode")}
                  className="rounded-control border border-border-strong bg-surface px-1.5 py-1 text-body"
                  value={privacy.privacy_failure_mode}
                  disabled={!privacy.bound || setPrivacy.isPending}
                  onChange={(event) =>
                    updatePrivacy({
                      privacyFailureMode: event.target.value as
                        | "block"
                        | "pass",
                    })
                  }
                >
                  <option value="block">{t("aicli.privacy.modeBlock")}</option>
                  <option value="pass">{t("aicli.privacy.modePass")}</option>
                </select>
              }
            />
            <SettingsSwitchRow
              label={t("aicli.settings.allowMedia")}
              description={t("aicli.settings.allowMediaHint")}
              checked={privacy.allow_unscanned_media}
              disabled={!privacy.bound || setPrivacy.isPending}
              onCheckedChange={(value) =>
                updatePrivacy({ allowUnscannedMedia: value })
              }
            />
            {privacy.bound && <CustomTerms privacy={privacy} />}
          </SettingsCard>
        )}
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
