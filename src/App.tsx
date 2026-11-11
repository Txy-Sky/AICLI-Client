import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "@/lib/toast";
import { invoke } from "@tauri-apps/api/core";
import { useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, ExternalLink, MoreHorizontal } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { VisibleApps } from "@/types";
import { KNOWN_APP_TYPES, type AppTypeFilter } from "@/types/usage";
import type { EnvConflict } from "@/types/env";
import { proxyKeys, useSettingsQuery } from "@/lib/query";
import { providersApi, settingsApi, type AppId } from "@/lib/api";
import { checkAllEnvConflicts, checkEnvConflicts } from "@/lib/api/env";
import { useOpenClawHealth } from "@/hooks/useOpenClaw";
import { useOpenHermesWebUI } from "@/hooks/useHermes";
import { hermesApi } from "@/lib/api/hermes";
import { useTauriEvent } from "@/hooks/useTauriEvent";
import { useAicliLineChanged } from "@/hooks/useAicliLines";
import { extractErrorMessage } from "@/utils/errorUtils";
import { isTextEditableTarget } from "@/utils/domUtils";
import { isLinux, isWindows } from "@/lib/platform";
import {
  APP_STORAGE_KEY,
  appPageBelongsTo,
  isAppPage,
  readStoredView,
  sharedFeatureAppOf,
  type GlobalPage,
  type SettingsSection,
  type View,
} from "@/lib/navigation";
import { Sidebar } from "@/components/shell/Sidebar";
import { useUpdate } from "@/contexts/UpdateContext";
import {
  AppPageHeader,
  WindowControlsContext,
} from "@/components/shell/AppPageHeader";
import { WindowControls } from "@/components/shell/WindowControls";
import { APP_DISPLAY_NAME, AppGlyph } from "@/components/shell/AppGlyph";
import { ProfileSwitcher } from "@/components/profiles/ProfileSwitcher";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { discardUnsavedChanges, hasUnsavedChanges } from "@/lib/unsavedChanges";
import { SettingsPage } from "@/components/settings/SettingsPage";
import { AppsPage } from "@/components/apps/AppsPage";
import {
  checkToolUpdatesInBackground,
  useToolUpdatesAvailable,
} from "@/components/apps/useToolManagement";
import { AicliHome } from "@/components/aicli/AicliHome";
import { AicliClientPage } from "@/components/aicli/AicliClientPage";
import { AicliClientsPage } from "@/components/aicli/clients/AicliClientsPage";
import { AicliPrivacyPage } from "@/components/aicli/privacy/AicliPrivacyPage";
import { useAicliEventBridge } from "@/hooks/useAicliAccount";
import { AICLI_EVENTS } from "@/lib/api/aicli";
import { AicliBindGate } from "@/components/aicli/AicliBindGate";
import { EnvWarningBanner } from "@/components/env/EnvWarningBanner";
import { proxyApi } from "@/lib/api/proxy";
import type { StartupAttachFailure } from "@/types/proxy";
import UnifiedMcpPanel from "@/components/mcp/UnifiedMcpPanel";
import PromptPanel from "@/components/prompts/PromptPanel";
import { PROMPT_APP_IDS } from "@/lib/query/prompts";
import UnifiedSkillsPanel from "@/components/skills/UnifiedSkillsPanel";
import { FirstRunNoticeDialog } from "@/components/FirstRunNoticeDialog";
import { WhatsNewNotice } from "@/components/WhatsNewDialog";
import { Button } from "@/components/ui/button";
import { HoverTip } from "@/components/ui/hover-tip";
import { PageTabs } from "@/components/ui/page-tabs";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { SessionManagerPage } from "@/components/sessions/SessionManagerPage";
import WorkspaceFilesPanel from "@/components/workspace/WorkspaceFilesPanel";
import EnvPanel from "@/components/openclaw/EnvPanel";
import ToolsPanel from "@/components/openclaw/ToolsPanel";
import AgentsDefaultsPanel from "@/components/openclaw/AgentsDefaultsPanel";
import OpenClawHealthBanner from "@/components/openclaw/OpenClawHealthBanner";
import HermesMemoryPanel, {
  HermesMemorySaveButton,
} from "@/components/hermes/HermesMemoryPanel";
import {
  APP_IDS,
  DEFAULT_VISIBLE_APPS,
  isModeAppId,
  orderAppIds,
} from "@/config/appConfig";

type OpenClawConfigTab = "env" | "tools" | "agents";

const getInitialApp = (): AppId => {
  const saved = localStorage.getItem(APP_STORAGE_KEY) as AppId | null;
  if (saved && APP_IDS.includes(saved)) {
    return saved;
  }
  return "claude";
};

function App() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const { hasUpdate } = useUpdate();

  const [activeApp, setActiveApp] = useState<AppId>(getInitialApp);
  const sharedFeatureApp = sharedFeatureAppOf(activeApp);
  const [currentView, setCurrentView] = useState<View>(readStoredView);
  const [settingsSection, setSettingsSection] =
    useState<SettingsSection>("general");
  const [appConfigScrollTarget, setAppConfigScrollTarget] = useState<AppId>();
  // 进设置前停留的页面：设置目录里的「← 返回」回到这里
  const settingsReturnViewRef = useRef<View>("client");
  const [openclawConfigTab, setOpenclawConfigTab] =
    useState<OpenClawConfigTab>("env");
  const [promptsApp, setPromptsApp] = useState<AppId>(() =>
    PROMPT_APP_IDS.includes(sharedFeatureApp) ? sharedFeatureApp : "claude",
  );
  const [, setConnectionStatusSlot] = useState<HTMLDivElement | null>(null);
  const [mcpManagementBusy, setMcpManagementBusy] = useState(false);
  const [skillsNavigationBusy, setSkillsNavigationBusy] = useState(false);
  const [promptNavigationBusy, setPromptNavigationBusy] = useState(false);

  const { data: settingsData } = useSettingsQuery();
  // Windows 一律去掉系统标题栏，用页头里的应用内窗口按钮；Linux 由设置决定
  const useAppWindowControls =
    isWindows() || (isLinux() && (settingsData?.useAppWindowControls ?? false));
  const visibleApps = useMemo<VisibleApps>(
    () => ({
      ...DEFAULT_VISIBLE_APPS,
      ...settingsData?.visibleApps,
    }),
    [settingsData?.visibleApps],
  );

  const getFirstVisibleApp = (): AppId => {
    return (
      orderAppIds(settingsData?.appOrder).find((app) => visibleApps[app]) ??
      "claude"
    );
  };

  useEffect(() => {
    if (!visibleApps[activeApp]) {
      setActiveApp(getFirstVisibleApp());
    }
  }, [visibleApps, activeApp]);

  // 「启动时检查应用更新」（默认关）：打开了才在后台查一次，查到新版本侧栏「应用」上出现圆点
  const checkToolUpdatesOnStartup =
    settingsData?.checkToolUpdatesOnStartup ?? false;
  const toolUpdatesAvailable = useToolUpdatesAvailable();
  const startupToolCheckDoneRef = useRef(false);
  useEffect(() => {
    if (!checkToolUpdatesOnStartup || startupToolCheckDoneRef.current) return;
    startupToolCheckDoneRef.current = true;
    void checkToolUpdatesInBackground();
  }, [checkToolUpdatesOnStartup]);

  // 启动时没能接上路由 / 聚合、已退回直连的应用：在对应的应用页提示一次并给「重试」
  const [, setStartupFailures] = useState<StartupAttachFailure[]>([]);
  useEffect(() => {
    proxyApi
      .takeStartupAttachFailures()
      .then((failures) => {
        if (failures?.length) setStartupFailures(failures);
      })
      .catch(() => undefined);
  }, []);

  // 应用专属页（工作区、记忆…）只属于它的应用；换了应用就回到供应商页
  useEffect(() => {
    if (isAppPage(currentView) && !appPageBelongsTo(currentView, activeApp)) {
      setCurrentView("client");
    }
  }, [activeApp, currentView]);

  const [envConflicts, setEnvConflicts] = useState<EnvConflict[]>([]);
  const [showEnvBanner, setShowEnvBanner] = useState(false);

  const mainScrollRef = useRef<HTMLElement>(null);

  // F-01：后端缓存刷新完成 → 对应的 AICLI 查询原地更新
  useAicliEventBridge();
  // F-07：深链 aicliapi://bind 打开绑定对话框（AicliBindGate）
  const [aicliBindRequest, setAicliBindRequest] = useState(0);
  useAicliLineChanged();
  useTauriEvent(AICLI_EVENTS.openBind, () => {
    const navigate = () => {
      setCurrentView("usage");
      setAicliBindRequest((count) => count + 1);
    };
    // 与托盘导航一致：编辑页有未保存修改时先确认
    if (managementBusyRef.current || !confirmLeave(navigate)) navigate();
  });

  useLayoutEffect(() => {
    if (currentView !== "client") return;

    for (const container of [mainScrollRef.current]) {
      if (container) {
        container.scrollTop = 0;
        container.scrollLeft = 0;
      }
    }
  }, [activeApp, currentView]);

  const modeAppId = isModeAppId(activeApp) ? activeApp : null;

  const isOpenClawView = activeApp === "openclaw" && isAppPage(currentView);
  const { data: openclawHealthWarnings = [] } =
    useOpenClawHealth(isOpenClawView);

  // 应用项目后刷新相关缓存（providers 由既有 provider-switched 监听承接；
  // proxy 状态由后端直接改 DB，不走 mutation，必须显式刷新）
  useTauriEvent("profile-applied", async () => {
    await queryClient.invalidateQueries({ queryKey: ["profiles"] });
    await queryClient.invalidateQueries({ queryKey: ["mcp", "all"] });
    await queryClient.invalidateQueries({ queryKey: ["skills"] });
    await queryClient.invalidateQueries({
      queryKey: proxyKeys.takeoverStatus,
    });
    await queryClient.invalidateQueries({ queryKey: proxyKeys.status });
    await queryClient.invalidateQueries({
      queryKey: ["providers", "claude-desktop"],
    });
  });

  useEffect(() => {
    // settingsData 未加载时跳过，避免用 fallback false 覆盖 Rust 侧已设好的装饰状态
    if (!settingsData) return;

    const syncWindowDecorations = async () => {
      try {
        await getCurrentWindow().setDecorations(!useAppWindowControls);
      } catch (error) {
        console.error("[App] Failed to update window decorations", error);
      }
    };

    void syncWindowDecorations();
  }, [useAppWindowControls, settingsData]);

  useEffect(() => {
    if (!isLinux() || !useAppWindowControls) return;

    const win = getCurrentWindow();
    let active = true;
    let revision = 0;
    let canResize = false;
    let nativeScale = 1;
    const unlisteners: (() => void)[] = [];
    let cursor:
      | {
          target: HTMLElement | SVGElement;
          value: string;
          priority: string;
          applied: string;
        }
      | undefined;
    const restoreCursor = () => {
      if (
        cursor &&
        cursor.target.style.cursor === cursor.applied &&
        cursor.target.style.getPropertyPriority("cursor") === "important"
      ) {
        cursor.target.style.setProperty(
          "cursor",
          cursor.value,
          cursor.priority,
        );
      }
      cursor = undefined;
    };
    const refresh = async () => {
      const current = ++revision;
      canResize = false;
      restoreCursor();
      try {
        const [maximized, fullscreen, resizable, scale] = await Promise.all([
          win.isMaximized(),
          win.isFullscreen(),
          win.isResizable(),
          win.scaleFactor(),
        ]);
        if (active && current === revision) {
          canResize = resizable && !maximized && !fullscreen;
          nativeScale = scale;
        }
      } catch (error) {
        console.error("[App] Failed to read window resize state", error);
      }
    };
    const edgeCursor = ({ clientX: x, clientY: y }: MouseEvent) => {
      if (
        !canResize ||
        x < 0 ||
        y < 0 ||
        x >= window.innerWidth ||
        y >= window.innerHeight
      ) {
        return "";
      }
      // 与 tauri-runtime-wry 的 GTK 边缘宽度（5 × scale，GDK 坐标）对齐，
      // 再从 GDK 坐标转换为 CSS 像素，兼容显示缩放和 WebView 缩放。
      const inset = (5 * nativeScale * nativeScale) / window.devicePixelRatio;
      const horizontal =
        x < inset ? -1 : x >= window.innerWidth - inset ? 1 : 0;
      const vertical = y < inset ? -1 : y >= window.innerHeight - inset ? 1 : 0;
      if (horizontal && vertical) {
        return horizontal === vertical ? "nwse-resize" : "nesw-resize";
      }
      return horizontal ? "ew-resize" : vertical ? "ns-resize" : "";
    };
    const onMouseMove = (event: MouseEvent) => {
      const value = edgeCursor(event);
      const target = event.target;
      if (
        !value ||
        !(target instanceof HTMLElement || target instanceof SVGElement)
      ) {
        restoreCursor();
        return;
      }
      if (cursor?.target !== target) {
        restoreCursor();
        cursor = {
          target,
          value: target.style.cursor,
          priority: target.style.getPropertyPriority("cursor"),
          applied: "",
        };
      }
      if (cursor.applied !== value) {
        cursor.applied = value;
        target.style.setProperty("cursor", value, "important");
      }
    };
    const onMouseDown = (event: MouseEvent) => {
      if (!(event.target instanceof Element)) return;
      const drag = event.target.getAttribute("data-tauri-drag-region");
      if (
        event.button === 0 &&
        drag !== null &&
        drag !== "false" &&
        edgeCursor(event)
      ) {
        // 保留 GTK 原生缩放，阻止 Tauri 的 document 冒泡监听再发起窗口移动。
        event.stopPropagation();
      }
    };

    document.addEventListener("mousemove", onMouseMove, true);
    document.addEventListener("mousedown", onMouseDown, true);
    document.documentElement.addEventListener("mouseleave", restoreCursor);
    window.addEventListener("blur", restoreCursor);
    for (const listener of [
      win.onResized(refresh),
      win.onScaleChanged(refresh),
    ]) {
      void listener
        .then((unlisten) => {
          if (active) unlisteners.push(unlisten);
          else unlisten();
        })
        .catch((error) =>
          console.error("[App] Failed to listen for window resize", error),
        );
    }
    void refresh();
    return () => {
      active = false;
      unlisteners.forEach((unlisten) => unlisten());
      document.removeEventListener("mousemove", onMouseMove, true);
      document.removeEventListener("mousedown", onMouseDown, true);
      document.documentElement.removeEventListener("mouseleave", restoreCursor);
      window.removeEventListener("blur", restoreCursor);
      restoreCursor();
    };
  }, [useAppWindowControls]);

  useEffect(() => {
    const checkEnvOnStartup = async () => {
      try {
        const allConflicts = await checkAllEnvConflicts();
        const flatConflicts = Object.values(allConflicts).flat();

        if (flatConflicts.length > 0) {
          setEnvConflicts(flatConflicts);
          const dismissed = sessionStorage.getItem("env_banner_dismissed");
          if (!dismissed) {
            setShowEnvBanner(true);
          }
        }
      } catch (error) {
        console.error(
          "[App] Failed to check environment conflicts on startup:",
          error,
        );
      }
    };

    checkEnvOnStartup();
  }, []);

  useEffect(() => {
    const checkMigration = async () => {
      try {
        const migrated = await invoke<boolean>("get_migration_result");
        if (migrated) {
          toast.success(
            t("migration.success", { defaultValue: "配置迁移成功" }),
            { closeButton: true },
          );
        }
      } catch (error) {
        console.error("[App] Failed to check migration result:", error);
      }
    };

    checkMigration();
  }, [t]);

  useEffect(() => {
    const checkSkillsMigration = async () => {
      try {
        const result = await invoke<{ count: number; error?: string } | null>(
          "get_skills_migration_result",
        );
        if (result?.error) {
          toast.error(t("migration.skillsFailed"), {
            description: t("migration.skillsFailedDescription"),
            closeButton: true,
          });
          console.error("[App] Skills SSOT migration failed:", result.error);
          return;
        }
        if (result && result.count > 0) {
          toast.success(t("migration.skillsSuccess", { count: result.count }), {
            closeButton: true,
          });
          await queryClient.invalidateQueries({ queryKey: ["skills"] });
        }
      } catch (error) {
        console.error("[App] Failed to check skills migration result:", error);
      }
    };

    checkSkillsMigration();
  }, [t, queryClient]);

  useEffect(() => {
    const checkEnvOnSwitch = async () => {
      try {
        if (
          activeApp === "mcode" ||
          activeApp === "dsh" ||
          activeApp === "kimi" ||
          activeApp === "agy"
        )
          return;
        const conflicts = await checkEnvConflicts(activeApp);

        if (conflicts.length > 0) {
          setEnvConflicts((prev) => {
            const existingKeys = new Set(
              prev.map((c) => `${c.varName}:${c.sourcePath}`),
            );
            const newConflicts = conflicts.filter(
              (c) => !existingKeys.has(`${c.varName}:${c.sourcePath}`),
            );
            return [...prev, ...newConflicts];
          });
          const dismissed = sessionStorage.getItem("env_banner_dismissed");
          if (!dismissed) {
            setShowEnvBanner(true);
          }
        }
      } catch (error) {
        console.error(
          "[App] Failed to check environment conflicts on app switch:",
          error,
        );
      }
    };

    checkEnvOnSwitch();
  }, [activeApp]);

  const currentViewRef = useRef(currentView);
  const managementBusy =
    mcpManagementBusy || skillsNavigationBusy || promptNavigationBusy;
  const managementBusyRef = useRef(false);
  managementBusyRef.current = managementBusy;

  useEffect(() => {
    currentViewRef.current = currentView;
  }, [currentView]);

  // ─── 导航 ───────────────────────────────────────────────────────────────
  // MCP / Skills / 提示词在进行中的操作会锁住导航（卸载面板会丢掉进行中的状态）

  // 整页编辑器里有未保存的修改：先确认，确认放弃后再执行这次导航
  const [pendingLeave, setPendingLeave] = useState<(() => void) | null>(null);
  const confirmLeave = (proceed: () => void) => {
    if (!hasUnsavedChanges()) return false;
    setPendingLeave(() => proceed);
    return true;
  };

  const openSettings = (
    section: SettingsSection = "general",
    appConfigTarget?: AppId,
  ) => {
    if (managementBusyRef.current) return;
    if (confirmLeave(() => openSettings(section, appConfigTarget))) return;
    if (currentViewRef.current !== "settings") {
      settingsReturnViewRef.current = currentViewRef.current;
    }
    setSettingsSection(section);
    setAppConfigScrollTarget(appConfigTarget);
    setCurrentView("settings");
  };

  const exitSettings = () => {
    setCurrentView(settingsReturnViewRef.current);
  };

  const selectApp = (app: AppId) => {
    if (managementBusyRef.current) return;
    if (confirmLeave(() => selectApp(app))) return;
    setActiveApp(app);
    localStorage.setItem(APP_STORAGE_KEY, app);
    setCurrentView("client");
  };

  const openPage = (page: GlobalPage | "settings") => {
    if (page === "settings") {
      openSettings("general");
      return;
    }
    if (managementBusyRef.current) return;
    // 再点当前页不会离开：不弹确认，也不能清掉编辑页的未保存登记
    if (currentViewRef.current === page) return;
    if (confirmLeave(() => openPage(page))) return;
    if (page === "prompts" && currentViewRef.current !== "prompts") {
      // 提示词页默认选中侧栏里最后选的那个应用
      setPromptsApp(
        PROMPT_APP_IDS.includes(sharedFeatureApp) ? sharedFeatureApp : "claude",
      );
    }
    setCurrentView(page);
  };

  // 侧栏、⌘K 进用量统计看全部应用；只有应用页 ⋯ 进来时带应用筛选
  const openPageFromNav = (page: GlobalPage | "settings") => {
    // 「设置」上的绿点说的是 CC Switch 有新版本，点进去直接到「关于」里的更新按钮
    if (page === "settings" && hasUpdate) {
      openSettings("about");
      return;
    }
    if (page === "usage") setUsageAppFilter("all");
    openPage(page);
  };

  const openSettingsRef = useRef(openSettings);
  openSettingsRef.current = openSettings;
  const [usageAppFilter, setUsageAppFilter] = useState<AppTypeFilter>("all");

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      const mod = event.metaKey || event.ctrlKey;
      if (mod && event.key === ",") {
        event.preventDefault();
        openSettingsRef.current("general");
        return;
      }

      if (event.key !== "Escape" || event.defaultPrevented) return;
      if (document.body.style.overflow === "hidden") return;
      if (managementBusyRef.current) return;
      if (isTextEditableTarget(event.target)) return;

      const view = currentViewRef.current;
      if (view === "skillsDiscovery") {
        event.preventDefault();
        setCurrentView("skills");
      } else if (view === "settings") {
        event.preventDefault();
        setCurrentView(settingsReturnViewRef.current);
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, []);

  const [launchDashboardOpen, setLaunchDashboardOpen] = useState(false);
  const openHermesWebUI = useOpenHermesWebUI(() =>
    setLaunchDashboardOpen(true),
  );

  const handleImportSuccess = async () => {
    try {
      await queryClient.invalidateQueries({
        queryKey: ["providers"],
        refetchType: "all",
      });
      await queryClient.refetchQueries({
        queryKey: ["providers"],
        type: "all",
      });
    } catch (error) {
      console.error("[App] Failed to refresh providers after import", error);
    }
    try {
      await providersApi.updateTrayMenu();
    } catch (error) {
      console.error("[App] Failed to refresh tray menu", error);
    }
  };

  const handleHideActiveApp = async () => {
    const visibleCount = Object.values(visibleApps).filter(Boolean).length;
    if (visibleCount <= 1) return;
    try {
      const current = await settingsApi.get();
      await settingsApi.save({
        ...current,
        visibleApps: { ...visibleApps, [activeApp]: false },
      });
      await queryClient.invalidateQueries({ queryKey: ["settings"] });
    } catch (error) {
      toast.error(
        t("settings.saveFailedGeneric", { defaultValue: "保存失败，请重试" }),
        { description: extractErrorMessage(error) || undefined },
      );
    }
  };

  // ─── 应用页 ─────────────────────────────────────────────────────────────

  // 应用页 ⋯ →「查看此应用的用量」：带上这个应用的筛选（Claude Desktop 的流量并在 Claude 里）
  const usageAppOf = (app: AppId): AppTypeFilter | null => {
    const type = sharedFeatureAppOf(app);
    return (KNOWN_APP_TYPES as ReadonlyArray<string>).includes(type)
      ? (type as AppTypeFilter)
      : null;
  };
  const activeUsageApp = usageAppOf(activeApp);

  const appMenu = (
    <DropdownMenu>
      <HoverTip content={t("common.more")}>
        <DropdownMenuTrigger asChild>
          <Button
            variant="quiet"
            size="icon-compact"
            className="h-8 w-8"
            aria-label={t("appPage.moreActions", {
              name: APP_DISPLAY_NAME[activeApp],
            })}
          >
            <MoreHorizontal className="h-4 w-4" />
          </Button>
        </DropdownMenuTrigger>
      </HoverTip>
      <DropdownMenuContent align="end" className="min-w-[180px]">
        <DropdownMenuItem onSelect={() => openPage("sessions")}>
          {t("appPage.viewSessions")}
        </DropdownMenuItem>
        {activeUsageApp && (
          <DropdownMenuItem
            onSelect={() => {
              setUsageAppFilter(activeUsageApp);
              openPage("usage");
            }}
          >
            {t("appPage.viewUsage")}
          </DropdownMenuItem>
        )}
        <DropdownMenuItem onSelect={() => openSettings("appConfig", activeApp)}>
          {t("appPage.configDirectory")}
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => openPage("apps")}>
          {t("appPage.installAndUpgrade")}
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          disabled={Object.values(visibleApps).filter(Boolean).length <= 1}
          onSelect={() => void handleHideActiveApp()}
        >
          {t("appPage.hideFromSidebar")}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );

  const appPageName =
    currentView === "workspace"
      ? t("appPage.workspace")
      : currentView === "openclawConfig"
        ? t("appPage.openclawConfig")
        : currentView === "hermesMemory"
          ? t("appPage.memory")
          : undefined;

  const renderAppPageHeader = () => (
    <AppPageHeader
      variant="app"
      // F-08：应用详情页从客户端总览进入，页头左侧给返回按钮
      leading={
        <HoverTip content={t("aicli.clients.back")}>
          <Button
            variant="quiet"
            size="icon-compact"
            className="h-8 w-8"
            aria-label={t("aicli.clients.back")}
            onClick={() => openPage("clients")}
          >
            <ArrowLeft className="h-4 w-4" />
          </Button>
        </HoverTip>
      }
      title={
        <span className="flex items-center gap-2">
          <AppGlyph app={activeApp} size={20} badgeClassName="bg-app" />
          {APP_DISPLAY_NAME[activeApp]}
        </span>
      }
      truncateTitle={currentView === "client" && Boolean(modeAppId)}
      titleExtra={
        currentView === "client" && modeAppId ? (
          <div
            ref={setConnectionStatusSlot}
            className="ms-2 flex min-w-0 max-w-[280px] items-center"
          />
        ) : undefined
      }
      subtitle={
        activeApp === "openclaw" ||
        activeApp === "hermes" ||
        (currentView === "client" && modeAppId)
          ? undefined
          : appPageName
      }
      actions={
        <>
          {currentView === "client" &&
            activeApp !== "mcode" &&
            activeApp !== "dsh" &&
            activeApp !== "kimi" &&
            (settingsData?.showProfileSwitcher ?? false) && (
              <ProfileSwitcher activeApp={activeApp} />
            )}
          {activeApp === "hermes" && (
            <Button
              variant="quiet"
              size="regular"
              onClick={() => void openHermesWebUI()}
            >
              {t("appPage.webUi")}
              <ExternalLink className="h-3.5 w-3.5" />
            </Button>
          )}
          {/* F-04：客户端页没有供应商列表，页头不再放搜索和「添加供应商」 */}
          {currentView === "hermesMemory" && <HermesMemorySaveButton />}
          {appMenu}
        </>
      }
    />
  );

  // OpenClaw / Hermes 没有模式 tab，那一行放它们自己的页面导航。
  // 换整块内容的是页面导航，用下划线页签；分段控件只留给模式和页内筛选。
  const renderAppSegments = () => {
    if (activeApp === "openclaw") {
      return (
        <div className="shrink-0 px-6 pt-2">
          <PageTabs
            aria-label={APP_DISPLAY_NAME.openclaw}
            idPrefix="openclaw-page"
            value={currentView}
            onValueChange={(view) => setCurrentView(view)}
            items={[
              { value: "client", label: t("appPage.client") },
              { value: "workspace", label: t("appPage.workspace") },
              { value: "openclawConfig", label: t("appPage.openclawConfig") },
            ]}
          />
        </div>
      );
    }
    if (activeApp === "hermes") {
      return (
        <div className="shrink-0 px-6 pt-2">
          <PageTabs
            aria-label={APP_DISPLAY_NAME.hermes}
            idPrefix="hermes-page"
            value={currentView}
            onValueChange={(view) => setCurrentView(view)}
            items={[
              { value: "client", label: t("appPage.client") },
              { value: "hermesMemory", label: t("appPage.memory") },
            ]}
          />
        </div>
      );
    }
    return null;
  };

  const renderAppPage = () => {
    const body = (() => {
      switch (currentView) {
        case "workspace":
          return <WorkspaceFilesPanel />;
        case "openclawConfig":
          return (
            <>
              {/* 二级页签：小一号、不画整行底线，挂在一级页签下面 */}
              <div className="shrink-0 px-6 pt-1">
                <PageTabs
                  size="sm"
                  aria-label={t("appPage.openclawConfig")}
                  idPrefix="openclaw-config"
                  controls="main-content"
                  value={openclawConfigTab}
                  onValueChange={setOpenclawConfigTab}
                  items={[
                    { value: "env", label: t("openclaw.env.title") },
                    { value: "tools", label: t("openclaw.tools.title") },
                    { value: "agents", label: t("openclaw.agents.title") },
                  ]}
                />
              </div>
              <div
                id="main-content"
                role="tabpanel"
                aria-labelledby={`openclaw-config-${openclawConfigTab}`}
                className="min-h-0 flex-1 overflow-y-auto scroll-stable"
              >
                {openclawConfigTab === "env" ? (
                  <EnvPanel />
                ) : openclawConfigTab === "tools" ? (
                  <ToolsPanel />
                ) : (
                  <AgentsDefaultsPanel />
                )}
              </div>
            </>
          );
        case "hermesMemory":
          return (
            <div
              id="main-content"
              className="min-h-0 flex-1 overflow-y-auto scroll-stable"
            >
              <HermesMemoryPanel onOpenWebUI={openHermesWebUI} />
            </div>
          );
        default:
          // F-04：客户端页（接管状态、配置集、模型表）取代原来的供应商列表
          return <AicliClientPage app={activeApp} />;
      }
    })();

    return (
      <>
        {renderAppPageHeader()}
        {renderAppSegments()}
        {isOpenClawView && openclawHealthWarnings.length > 0 && (
          <OpenClawHealthBanner warnings={openclawHealthWarnings} />
        )}
        {body}
      </>
    );
  };

  // ─── 全局页 ─────────────────────────────────────────────────────────────

  const renderGlobalPage = () => {
    switch (currentView) {
      case "clients":
        return (
          <AicliClientsPage
            visibleApps={visibleApps}
            appOrder={settingsData?.appOrder}
            onOpenApp={selectApp}
            onOpenApps={() => openPage("apps")}
          />
        );
      case "privacy":
        return <AicliPrivacyPage />;
      case "usage":
        return (
          <AicliHome
            initialAppType={usageAppFilter}
            onOpenRoutingSettings={() => openSettings("routing")}
            onOpenPrivacy={() => openPage("privacy")}
          />
        );
      case "mcp":
        return (
          <UnifiedMcpPanel onInteractionBlockedChange={setMcpManagementBusy} />
        );
      case "skills":
      case "skillsDiscovery":
        // 「已安装 / 发现」合成一页；旧的 skillsDiscovery 视图直接打开「发现」段
        return (
          <UnifiedSkillsPanel
            initialView={
              currentView === "skillsDiscovery" ? "discover" : "installed"
            }
            onNavigationBlockedChange={setSkillsNavigationBusy}
          />
        );
      case "prompts":
        return (
          <div id="main-content" className="flex min-h-0 flex-1 flex-col">
            <PromptPanel
              appId={promptsApp}
              apps={PROMPT_APP_IDS.filter(
                (app) =>
                  visibleApps[app] ||
                  (app === "claude" && visibleApps["claude-desktop"]) ||
                  app === promptsApp,
              )}
              onAppChange={setPromptsApp}
              onNavigationBlockedChange={setPromptNavigationBusy}
            />
          </div>
        );
      case "sessions":
        // 页头（含 ⋯ 菜单）由会话页自己画：菜单里的操作都在页面状态里
        return (
          <SessionManagerPage
            key={sharedFeatureApp}
            appId={sharedFeatureApp}
            fromApp={activeApp}
            onOpenTerminalSettings={() => openSettings("general")}
          />
        );
      case "apps":
        return <AppsPage />;
      default:
        return null;
    }
  };

  const renderContent = () => {
    if (currentView === "settings") {
      return (
        <SettingsPage
          section={settingsSection}
          appConfigScrollTarget={appConfigScrollTarget}
          onImportSuccess={handleImportSuccess}
          onOpenApps={() => setCurrentView("apps")}
          onOpenApp={selectApp}
          onOpenPrivacy={() => openPage("privacy")}
        />
      );
    }
    return isAppPage(currentView) ? renderAppPage() : renderGlobalPage();
  };

  return (
    <WindowControlsContext.Provider
      value={useAppWindowControls ? <WindowControls /> : null}
    >
      <AicliBindGate bindRequest={aicliBindRequest}>
        <div className="flex h-screen overflow-hidden bg-app text-fg-1 selection:bg-action/25">
          <Sidebar
            activeApp={activeApp}
            view={currentView}
            visibleApps={visibleApps}
            settingsSection={settingsSection}
            onSelectPage={openPageFromNav}
            onSelectSettingsSection={(section) => {
              setAppConfigScrollTarget(undefined);
              setSettingsSection(section);
            }}
            onExitSettings={exitSettings}
            appsUpdateAvailable={
              checkToolUpdatesOnStartup && toolUpdatesAvailable
            }
          />
          <main
            ref={mainScrollRef}
            id="content-area"
            className="relative isolate flex min-w-0 flex-1 flex-col overflow-hidden bg-app"
          >
            {showEnvBanner && envConflicts.length > 0 && (
              <EnvWarningBanner
                conflicts={envConflicts}
                onDismiss={() => {
                  setShowEnvBanner(false);
                  sessionStorage.setItem("env_banner_dismissed", "true");
                }}
                onDeleted={async () => {
                  try {
                    const allConflicts = await checkAllEnvConflicts();
                    const flatConflicts = Object.values(allConflicts).flat();
                    setEnvConflicts(flatConflicts);
                    if (flatConflicts.length === 0) {
                      setShowEnvBanner(false);
                    }
                  } catch (error) {
                    console.error(
                      "[App] Failed to re-check conflicts after deletion:",
                      error,
                    );
                  }
                }}
              />
            )}
            <div
              key={
                currentView === "settings"
                  ? "settings"
                  : isAppPage(currentView)
                    ? "app"
                    : currentView
              }
              className="flex min-h-0 flex-1 flex-col"
            >
              {renderContent()}
            </div>
          </main>
        </div>

        <ConfirmDialog
          isOpen={pendingLeave !== null}
          title={t("common.unsavedLeaveTitle")}
          message={t("common.unsavedLeaveMessage")}
          confirmText={t("common.unsavedLeaveConfirm")}
          cancelText={t("common.unsavedLeaveCancel")}
          zIndex="top"
          onConfirm={() => {
            const proceed = pendingLeave;
            setPendingLeave(null);
            discardUnsavedChanges();
            proceed?.();
          }}
          onCancel={() => setPendingLeave(null)}
        />

        <ConfirmDialog
          isOpen={launchDashboardOpen}
          title={t("hermes.webui.launchConfirmTitle")}
          message={t("hermes.webui.launchConfirmMessage")}
          confirmText={t("hermes.webui.launchConfirmAction")}
          variant="info"
          onConfirm={() => {
            setLaunchDashboardOpen(false);
            void (async () => {
              try {
                await hermesApi.launchDashboard();
                toast.success(t("hermes.webui.launching"));
              } catch (error) {
                toast.error(t("hermes.webui.launchFailed"), {
                  description: extractErrorMessage(error) || undefined,
                });
              }
            })();
          }}
          onCancel={() => setLaunchDashboardOpen(false)}
        />

        <FirstRunNoticeDialog />
        <WhatsNewNotice />
      </AicliBindGate>
    </WindowControlsContext.Provider>
  );
}

export default App;
