import React, { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, ChevronRight, Loader2 } from "lucide-react";
import { toast } from "@/lib/toast";
import { Button } from "@/components/ui/button";
import { Notice } from "@/components/ui/notice";
import {
  DISCOVER_VIEW_KEYS,
  DiscoverViewToggle,
  useDiscoverView,
} from "@/components/discover/DiscoverViewToggle";
import { DiscoverGrid } from "@/components/discover/DiscoverGrid";
import { DiscoverCard } from "@/components/discover/DiscoverCard";
import { DiscoverLogo } from "@/components/discover/DiscoverLogo";
import { mcpCatalog } from "@/config/mcpCatalog";
import type { McpAppId } from "@/config/appConfig";
import { useSearchMcpRegistry, useUpsertMcpServer } from "@/hooks/useMcp";
import {
  mergeDiscoverResults,
  needsInput,
  serverKey,
  type McpCatalogEntry,
} from "@/lib/mcpRegistry";
import type { McpServersMap } from "@/types";
import {
  extractErrorMessage,
  translateMcpBackendError,
} from "@/utils/errorUtils";
import { cn } from "@/lib/utils";
import { MatrixSearch } from "./AppMatrix";
import { uniqueId } from "./mcpDraft";
import {
  defaultDiscoverApps,
  discoverDescription,
  discoverServerOf,
  type DiscoverApps,
} from "./mcpDiscoverEntry";
import { DiscoverMeta } from "./McpDiscoverMeta";
import { McpDiscoverAddDialog } from "./McpDiscoverAddDialog";

/** 输入停顿多久后去 Registry 搜索 */
const SEARCH_DEBOUNCE_MS = 320;

const entryKey = (entry: McpCatalogEntry) =>
  `${entry.source}:${entry.registryName ?? entry.id}`;

interface McpDiscoverProps {
  /** 页签行右侧的槽（搜索框放这里） */
  renderViewTabs: (trailing: React.ReactNode) => React.ReactNode;
  existingServers: McpServersMap;
  visibleAppIds: McpAppId[];
  /** 页面有写入 / 对话框进行中时不让添加 */
  disabled?: boolean;
  /** 「已添加 ›」：切到已安装段并按键名搜索 */
  onShowInstalled: (id: string) => void;
}

/**
 * MCP 的「发现」段：精选（src/config/mcpCatalog.ts）+ 搜索官方 MCP Registry。
 * 列表 / 卡片两种视图；不需要填写的直接「添加」，要填密钥的「添加…」先弹表单。
 */
export function McpDiscover({
  renderViewTabs,
  existingServers,
  visibleAppIds,
  disabled,
  onShowInstalled,
}: McpDiscoverProps) {
  const { t } = useTranslation();
  const [view, setView] = useDiscoverView(DISCOVER_VIEW_KEYS.mcp, "grid");
  const [query, setQuery] = useState("");
  const [searchQuery, setSearchQuery] = useState("");
  const [adding, setAdding] = useState<Set<string>>(new Set());
  const [dialog, setDialog] = useState<McpCatalogEntry | null>(null);
  const upsert = useUpsertMcpServer();

  // 停顿后再搜，回车立即搜
  useEffect(() => {
    const handle = window.setTimeout(
      () => setSearchQuery(query.trim()),
      SEARCH_DEBOUNCE_MS,
    );
    return () => window.clearTimeout(handle);
  }, [query]);

  const registry = useSearchMcpRegistry(searchQuery);
  const searching = searchQuery.length >= 2;

  const normalized = query.trim().toLowerCase();
  const featured = useMemo(() => {
    if (!normalized) return mcpCatalog;
    return mcpCatalog.filter((entry) =>
      [
        entry.id,
        entry.title,
        entry.publisher,
        entry.description,
        discoverDescription(entry, t),
      ].some((value) => value.toLowerCase().includes(normalized)),
    );
  }, [normalized, t]);

  const entries = useMemo(
    () =>
      searching
        ? mergeDiscoverResults(featured, registry.data ?? [], searchQuery)
        : featured,
    [featured, registry.data, searchQuery, searching],
  );

  /** 已经在资源库里的：按地址 / 命令判断（键名可能不同） */
  const haveByKey = useMemo(() => {
    const map = new Map<string, string>();
    for (const [id, server] of Object.entries(existingServers)) {
      const key = serverKey(server.server);
      if (key && !map.has(key)) map.set(key, id);
    }
    return map;
  }, [existingServers]);
  const haveOf = (entry: McpCatalogEntry) =>
    haveByKey.get(serverKey(entry.spec));

  const existingIds = useMemo(
    () => Object.keys(existingServers),
    [existingServers],
  );

  const add = async (
    entry: McpCatalogEntry,
    spec: McpCatalogEntry["spec"],
    apps: DiscoverApps,
  ) => {
    const key = entryKey(entry);
    if (adding.has(key)) return false;
    const id = uniqueId(entry.id, existingIds);
    setAdding((prev) => new Set(prev).add(key));
    try {
      await upsert.mutateAsync(
        discoverServerOf(entry, id, spec, apps, discoverDescription(entry, t)),
      );
      toast.success(t("mcpPage.toast.added", { id }), { closeButton: true });
      return true;
    } catch (error) {
      const detail = extractErrorMessage(error);
      toast.error(
        translateMcpBackendError(detail, t) ||
          detail ||
          t("mcp.error.saveFailed"),
        { duration: 6000 },
      );
      return false;
    } finally {
      setAdding((prev) => {
        const next = new Set(prev);
        next.delete(key);
        return next;
      });
    }
  };

  const quickAdd = (entry: McpCatalogEntry) =>
    void add(entry, entry.spec, defaultDiscoverApps(visibleAppIds, entry.spec));

  const open = (entry: McpCatalogEntry) => {
    if (disabled) return;
    setDialog(entry);
  };

  const renderAction = (entry: McpCatalogEntry) => {
    const have = haveOf(entry);
    if (have) {
      return (
        <button
          type="button"
          onClick={() => onShowInstalled(have)}
          aria-label={t("mcpPage.discover.addedAria", { name: entry.title })}
          title={t("mcpPage.discover.addedAs", { id: have })}
          className="inline-flex h-7 items-center gap-1 rounded-control px-2 text-body text-fg-2 transition-colors hover:bg-subtle hover:text-fg-1"
        >
          <Check className="h-3.5 w-3.5" strokeWidth={2} />
          {t("mcpPage.discover.added")}
          <ChevronRight className="h-3.5 w-3.5" strokeWidth={2} />
        </button>
      );
    }
    if (adding.has(entryKey(entry))) {
      return (
        <Button
          type="button"
          variant="neutral"
          size="compact"
          aria-disabled="true"
          className="cursor-not-allowed ps-2.5"
        >
          <Loader2 className="h-3.5 w-3.5 animate-spin" />
          {t("mcpPage.discover.adding")}
        </Button>
      );
    }
    const ask = needsInput(entry);
    return (
      <Button
        type="button"
        variant="neutral"
        size="compact"
        disabled={disabled}
        aria-label={t("mcpPage.discover.addAria", { name: entry.title })}
        onClick={() => (ask ? open(entry) : quickAdd(entry))}
      >
        {ask ? t("mcpPage.discover.addEllipsis") : t("mcpPage.discover.add")}
      </Button>
    );
  };

  const renderCard = (entry: McpCatalogEntry) => {
    const description = discoverDescription(entry, t);
    return (
      <DiscoverCard
        logo={<DiscoverLogo src={entry.icon} name={entry.title} />}
        title={entry.title}
        openLabel={t("mcpPage.discover.openAria", { name: entry.title })}
        onOpen={disabled ? undefined : () => open(entry)}
        meta={<DiscoverMeta entry={entry} />}
        description={description}
        descriptionTitle={description || undefined}
        dimmed={Boolean(haveOf(entry))}
        footLeft={
          <span className="min-w-0 truncate font-mono" title={entry.id}>
            {entry.id}
          </span>
        }
        action={renderAction(entry)}
      />
    );
  };

  const renderRow = (entry: McpCatalogEntry, index: number) => {
    const description = discoverDescription(entry, t);
    return (
      <li
        key={entryKey(entry)}
        className={cn(
          "flex h-14 items-center gap-3 pe-3 ps-4",
          index > 0 && "border-t border-border",
        )}
      >
        <DiscoverLogo src={entry.icon} name={entry.title} size={26} />
        <div className="flex min-w-0 flex-1 flex-col">
          <div className="flex min-w-0 items-center gap-1.5 text-caption text-fg-2">
            <button
              type="button"
              disabled={disabled}
              onClick={() => open(entry)}
              aria-label={t("mcpPage.discover.openAria", { name: entry.title })}
              className="min-w-0 truncate rounded-[4px] text-left text-body font-medium text-fg-1 hover:underline focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:no-underline"
            >
              {entry.title}
            </button>
            <DiscoverMeta entry={entry} />
          </div>
          <span className="truncate text-caption text-fg-2" title={description}>
            {description}
          </span>
        </div>
        <div className="flex shrink-0 justify-end">{renderAction(entry)}</div>
      </li>
    );
  };

  const registryFailed = searching && registry.isError;
  const loading = searching && registry.isFetching;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {renderViewTabs(
        <MatrixSearch
          className="w-[300px] min-w-[180px] shrink"
          inputId="mcp-discover-search"
          value={query}
          onValueChange={setQuery}
          onEnter={() => setSearchQuery(query.trim())}
          placeholder={t("mcpPage.discover.searchPlaceholder")}
          ariaLabel={t("mcpPage.discover.searchAria")}
          status={
            normalized
              ? t("mcpPage.discover.results", { count: entries.length })
              : ""
          }
        />,
      )}

      <div className="flex h-12 shrink-0 items-center gap-2 px-6">
        <span className="flex min-w-0 items-center gap-1.5 truncate text-caption text-fg-2">
          {loading && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
          {loading
            ? t("mcpPage.discover.searching")
            : normalized
              ? t("mcpPage.discover.results", { count: entries.length })
              : t("mcpPage.discover.sourceNote")}
        </span>
        <div className="flex-1" />
        <DiscoverViewToggle value={view} onValueChange={setView} />
      </div>

      {registryFailed && (
        <div className="px-6 pb-3">
          <Notice
            tone="warning"
            title={t("mcpPage.discover.registryFailedTitle")}
            actions={
              <Button
                type="button"
                variant="neutral"
                size="compact"
                disabled={registry.isFetching}
                onClick={() => void registry.refetch()}
              >
                {t("common.retry")}
              </Button>
            }
          >
            {t("mcpPage.discover.registryFailedBody", {
              error: extractErrorMessage(registry.error) || "",
            })}
          </Notice>
        </div>
      )}

      <div className="flex min-h-0 flex-1 flex-col px-6 pb-5">
        {entries.length === 0 ? (
          <div className="flex flex-col items-center gap-2 rounded-panel border border-border bg-surface px-6 py-10 text-center">
            {loading ? (
              <span className="flex items-center gap-2 text-body text-fg-2">
                <Loader2 className="h-4 w-4 animate-spin" />
                {t("mcpPage.discover.searching")}
              </span>
            ) : (
              <>
                <h2 className="m-0 text-section">
                  {t("mcpPage.discover.empty", { query: query.trim() })}
                </h2>
                <p className="m-0 max-w-[460px] text-body text-fg-2">
                  {t("mcpPage.discover.emptyHint")}
                </p>
                <Button
                  type="button"
                  variant="neutral"
                  size="regular"
                  onClick={() => setQuery("")}
                >
                  {t("mcpPage.clearSearch")}
                </Button>
              </>
            )}
          </div>
        ) : view === "grid" ? (
          <DiscoverGrid
            ariaLabel={t("mcpPage.discover.listLabel")}
            items={entries}
            getKey={entryKey}
            renderCard={renderCard}
            className={cn(loading && "opacity-60 transition-opacity")}
          />
        ) : (
          <div
            className={cn(
              "min-h-0 overflow-auto scroll-stable rounded-panel border border-border bg-surface",
              loading && "opacity-60 transition-opacity",
            )}
          >
            <ul
              aria-label={t("mcpPage.discover.listLabel")}
              className="m-0 min-w-[520px] list-none p-0"
            >
              {entries.map(renderRow)}
            </ul>
          </div>
        )}
      </div>

      {dialog && (
        <McpDiscoverAddDialog
          entry={dialog}
          id={uniqueId(dialog.id, existingIds)}
          visibleAppIds={visibleAppIds}
          pending={adding.has(entryKey(dialog))}
          onCancel={() => setDialog(null)}
          onConfirm={(spec, apps) => {
            void add(dialog, spec, apps).then((ok) => {
              if (ok) setDialog(null);
            });
          }}
        />
      )}
    </div>
  );
}
