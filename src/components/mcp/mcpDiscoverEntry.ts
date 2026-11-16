import type { TFunction } from "i18next";
import type { McpServer, McpServerSpec } from "@/types";
import { MCP_APP_IDS, type McpAppId } from "@/config/appConfig";
import type { McpCatalogEntry, McpCatalogInput } from "@/lib/mcpRegistry";
import { SSE_REJECTED_APPS, transportOf } from "./mcpDraft";

export type DiscoverApps = Record<McpAppId, boolean>;

/** 默认写入到哪些应用：和添加抽屉一样，界面上显示的应用全勾；SSE 时去掉不收 SSE 的应用 */
export function defaultDiscoverApps(
  visibleAppIds: readonly McpAppId[],
  spec: McpServerSpec,
): DiscoverApps {
  const sse = transportOf(spec) === "sse";
  const out = {} as DiscoverApps;
  for (const app of MCP_APP_IDS) {
    out[app] =
      visibleAppIds.includes(app) && !(sse && SSE_REJECTED_APPS.includes(app));
  }
  return out;
}

/** 发现栏条目 → 要写入的 MCP 服务器（键名、说明、主页一并带上） */
export function discoverServerOf(
  entry: McpCatalogEntry,
  id: string,
  spec: McpServerSpec,
  apps: DiscoverApps,
  description: string,
): McpServer {
  const server: McpServer = {
    id,
    name: entry.title || id,
    server: spec,
    apps: { openclaw: false, ...apps } as McpServer["apps"],
  };
  if (description) server.description = description;
  if (entry.homepage) server.homepage = entry.homepage;
  return server;
}

/** 精选条目用译文，Registry 条目用原文 */
export function discoverDescription(entry: McpCatalogEntry, t: TFunction) {
  return entry.source === "featured"
    ? t(`mcpCatalog.${entry.id}`, { defaultValue: entry.description })
    : entry.description;
}

export function inputLabel(input: McpCatalogInput, t: TFunction): string {
  return input.labelKey
    ? t(`mcpPage.discover.labels.${input.labelKey}`)
    : input.label || input.key;
}
