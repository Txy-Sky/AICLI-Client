import { useTranslation } from "react-i18next";
import { KeyRound } from "lucide-react";
import { needsInput, type McpCatalogEntry } from "@/lib/mcpRegistry";
import { NeutralBadge } from "./AppMatrix";
import { inputLabel } from "./mcpDiscoverEntry";

/** 卡片 / 行 / 对话框的副行：发布者 + 运行方式（npx / uvx / 远程）+ 要填密钥时的钥匙 */
export function DiscoverMeta({ entry }: { entry: McpCatalogEntry }) {
  const { t } = useTranslation();
  const runs = entry.runs
    ? entry.runs
    : entry.signIn
      ? t("mcpPage.discover.remoteSignIn")
      : t("mcpPage.discover.remote");
  const required = entry.inputs.filter((input) => input.required);
  return (
    <>
      <span className="min-w-0 truncate" title={entry.publisher}>
        {entry.publisher}
      </span>
      <NeutralBadge mono={Boolean(entry.runs)}>{runs}</NeutralBadge>
      {needsInput(entry) && (
        <span
          role="img"
          aria-label={t("mcpPage.discover.needsKey", {
            labels: required
              .map((input) => inputLabel(input, t))
              .join(t("mcpPage.listSeparator")),
          })}
          title={t("mcpPage.discover.needsKey", {
            labels: required
              .map((input) => inputLabel(input, t))
              .join(t("mcpPage.listSeparator")),
          })}
          className="inline-flex h-[18px] w-[18px] shrink-0 items-center justify-center rounded-full border border-border-strong text-fg-2"
        >
          <KeyRound className="h-2.5 w-2.5" strokeWidth={2} />
        </span>
      )}
    </>
  );
}
