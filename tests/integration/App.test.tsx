import { Suspense, type ComponentType } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  act,
  render,
  screen,
  waitFor,
  fireEvent,
  within,
} from "@testing-library/react";
import { describe, it, expect, beforeEach, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { server } from "../msw/server";
import { resetProviderState, setSettings } from "../msw/state";
import { emitTauriEvent } from "../msw/tauriMocks";

// The heaviest App integration cases kept a 10s budget on Linux; Windows
// jsdom is several times slower (docs/aicli-handover/E-GATE.md).
const APP_TEST_TIMEOUT = process.platform === "win32" ? 180_000 : 10_000;

const toastSuccessMock = vi.fn();
const toastErrorMock = vi.fn();
const skillsPanelMocks = vi.hoisted(() => ({
  initialViews: [] as string[],
}));
const windowMocks = vi.hoisted(() => ({
  isLinux: vi.fn(() => false),
  isMaximized: vi.fn(),
  isFullscreen: vi.fn(),
  isResizable: vi.fn(),
  scaleFactor: vi.fn(),
  setDecorations: vi.fn(async () => {}),
  resizeListeners: new Set<() => void>(),
  onResized: vi.fn(),
  onScaleChanged: vi.fn(),
}));

vi.mock("@/lib/platform", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/platform")>()),
  isLinux: windowMocks.isLinux,
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => windowMocks,
}));

vi.mock("sonner", () => ({
  toast: {
    success: (...args: unknown[]) => toastSuccessMock(...args),
    error: (...args: unknown[]) => toastErrorMock(...args),
  },
}));

// G-01：供应商列表、添加 / 编辑对话框、用量脚本已删除，不再需要替身

vi.mock("@/components/ConfirmDialog", () => ({
  ConfirmDialog: ({ isOpen, message, onConfirm, onCancel }: any) =>
    isOpen ? (
      <div data-testid="confirm-dialog">
        <div data-testid="confirm-message">{message}</div>
        <button onClick={() => onConfirm()}>confirm-delete</button>
        <button onClick={() => onCancel()}>cancel-delete</button>
      </div>
    ) : null,
}));

vi.mock("@/contexts/UpdateContext", () => ({
  useUpdate: () => ({ hasUpdate: false, updateInfo: null }),
}));

// 设置页要 ThemeProvider，这里只看导航进去时发生了什么
vi.mock("@/components/settings/SettingsPage", () => ({
  SettingsPage: () => <div data-testid="settings-page" />,
}));

vi.mock("@/components/skills/UnifiedSkillsPanel", () => ({
  // v7：Skills 的页头（添加 / 检查更新 / 存储与同步）在面板自己里面
  default: ({ initialView }: { initialView?: string }) => {
    skillsPanelMocks.initialViews.push(initialView ?? "installed");
    return <div data-testid="unified-skills-panel">{initialView}</div>;
  },
}));

vi.mock("@/components/mcp/McpPanel", () => ({
  default: ({ open, onOpenChange }: any) =>
    open ? (
      <div data-testid="mcp-panel">
        <button onClick={() => onOpenChange(false)}>close-mcp</button>
      </div>
    ) : (
      <button onClick={() => onOpenChange(true)}>open-mcp</button>
    ),
}));

vi.mock("@/components/mcp/UnifiedMcpPanel", async () => {
  const { useUnsavedChangesTracker } = await vi.importActual<
    typeof import("@/lib/unsavedChanges")
  >("@/lib/unsavedChanges");
  // 当作一直开着的 MCP 编辑页：输入即登记未保存的修改
  const Body = () => (
    <div data-testid="mcp-editor" {...useUnsavedChangesTracker()}>
      <input aria-label="mcp-field" />
    </div>
  );
  return { default: Body };
});

/** 侧栏里的应用行（v7 侧栏取代了原来页头的应用切换器） */
const sidebarApp = (name: string) =>
  // 首次启动提示是模态对话框，会把侧栏标成 aria-hidden，所以带上 hidden
  within(document.querySelector("nav") as HTMLElement).getByRole("button", {
    name,
    hidden: true,
  });

/**
 * F-02：冷启动恒停在用量页。要看某个应用的客户端页，先点侧栏里它的应用行
 * （应用本身仍按 aicli-client-last-app 恢复）。
 */
const enterClientPage = (name: string) => fireEvent.click(sidebarApp(name));

const renderApp = (AppComponent: ComponentType) => {
  const client = new QueryClient();
  return render(
    <QueryClientProvider client={client}>
      <Suspense fallback={<div data-testid="loading">loading</div>}>
        <AppComponent />
      </Suspense>
    </QueryClientProvider>,
  );
};

// F-07：AicliBindGate 读完绑定状态才渲染主界面；先等侧栏出现再操作
const renderAppReady = async (AppComponent: ComponentType) => {
  const view = renderApp(AppComponent);
  await screen.findByRole("navigation", { hidden: true }, { timeout: 5000 });
  return view;
};

const boundAccount = {
  state: "bound",
  user_id: 7,
  username: "alice",
  user_group: "default",
  selected_line_id: "main",
  active_profile_id: null,
  managed_tokens: [],
  quota_display: null,
};

// F-07：未绑定时 AicliBindGate 不让进入应用；集成用例默认在已绑定状态下跑
const useBoundAccount = () =>
  server.use(
    http.post("http://tauri.local/aicli_get_account", () =>
      HttpResponse.json(boundAccount),
    ),
  );

describe("App integration with MSW", () => {
  beforeEach(() => {
    useBoundAccount();
    resetProviderState();
    toastSuccessMock.mockReset();
    toastErrorMock.mockReset();
    skillsPanelMocks.initialViews = [];
    windowMocks.isLinux.mockReturnValue(false);
    windowMocks.isMaximized.mockResolvedValue(false);
    windowMocks.isFullscreen.mockResolvedValue(false);
    windowMocks.isResizable.mockResolvedValue(true);
    windowMocks.scaleFactor.mockResolvedValue(1);
    windowMocks.resizeListeners.clear();
    const listen = async (handler: () => void) => {
      windowMocks.resizeListeners.add(handler);
      return () => windowMocks.resizeListeners.delete(handler);
    };
    windowMocks.onResized.mockImplementation(listen);
    windowMocks.onScaleChanged.mockImplementation(listen);
    localStorage.removeItem("aicli-client-last-view");
    localStorage.removeItem("aicli-client-last-app");
  });

  // F-04：供应商编辑面板随供应商列表移除，未保存保护改用 MCP 编辑页（同一套
  // useUnsavedChangesTracker）验证
  it(
    "asks before leaving an editor page with unsaved changes",
    async () => {
      const { default: App } = await import("@/App");
      await renderAppReady(App);

      // 没改过：照常离开，不问
      fireEvent.click(sidebarApp("MCP"));
      expect(await screen.findByTestId("mcp-editor")).toBeInTheDocument();
      fireEvent.click(sidebarApp("nav.usage"));
      await waitFor(() =>
        expect(screen.queryByTestId("mcp-editor")).not.toBeInTheDocument(),
      );
      expect(screen.queryByTestId("confirm-dialog")).not.toBeInTheDocument();

      // 改过：切应用先问；继续编辑就留在原页
      fireEvent.click(sidebarApp("MCP"));
      fireEvent.input(await screen.findByLabelText("mcp-field"), {
        target: { value: "draft" },
      });
      fireEvent.click(sidebarApp("Codex"));
      expect(await screen.findByTestId("confirm-message")).toHaveTextContent(
        "common.unsavedLeaveMessage",
      );
      fireEvent.click(screen.getByText("cancel-delete"));
      expect(screen.getByTestId("mcp-editor")).toBeInTheDocument();

      // ⌘, 也问；放弃后才离开
      fireEvent.keyDown(window, { key: ",", metaKey: true });
      expect(await screen.findByTestId("confirm-message")).toHaveTextContent(
        "common.unsavedLeaveMessage",
      );
      fireEvent.click(screen.getByText("confirm-delete"));
      expect(await screen.findByTestId("settings-page")).toBeInTheDocument();
      expect(screen.queryByTestId("mcp-editor")).not.toBeInTheDocument();
    },
    APP_TEST_TIMEOUT,
  );

  it(
    "keeps the unsaved guard when re-clicking the current page",
    async () => {
      const { default: App } = await import("@/App");
      await renderAppReady(App);

      fireEvent.click(sidebarApp("MCP"));
      fireEvent.input(await screen.findByLabelText("mcp-field"), {
        target: { value: "draft" },
      });

      // 再点当前页：不离开，所以不问，也不能把未保存登记清掉
      fireEvent.click(sidebarApp("MCP"));
      expect(screen.queryByTestId("confirm-dialog")).not.toBeInTheDocument();
      expect(screen.getByTestId("mcp-editor")).toBeInTheDocument();

      // 之后真的离开时照样要问
      fireEvent.click(sidebarApp("nav.usage"));
      expect(await screen.findByTestId("confirm-message")).toHaveTextContent(
        "common.unsavedLeaveMessage",
      );
      fireEvent.click(screen.getByText("cancel-delete"));
      expect(screen.getByTestId("mcp-editor")).toBeInTheDocument();
    },
    APP_TEST_TIMEOUT,
  );

  it("shows toast when auto sync fails in background", async () => {
    const { default: App } = await import("@/App");
    await renderAppReady(App);

    // 等首页渲染完（监听在挂载后异步注册）
    await waitFor(() =>
      expect(sidebarApp("nav.usage")).toHaveAttribute("aria-current", "page"),
    );
    await act(async () => {
      await Promise.resolve();
    });

    expect(() => {
      emitTauriEvent("webdav-sync-status-updated", null);
    }).not.toThrow();
    expect(toastErrorMock).not.toHaveBeenCalled();

    emitTauriEvent("webdav-sync-status-updated", {
      source: "auto",
      status: "error",
      error: "network timeout",
    });

    await waitFor(() => {
      expect(toastErrorMock).toHaveBeenCalled();
    });

    toastErrorMock.mockReset();
    expect(() => {
      emitTauriEvent("s3-sync-status-updated", null);
    }).not.toThrow();
    expect(toastErrorMock).not.toHaveBeenCalled();

    emitTauriEvent("s3-sync-status-updated", {
      source: "auto",
      status: "error",
      error: "s3 timeout",
    });

    await waitFor(() => {
      expect(toastErrorMock).toHaveBeenCalled();
    });
  });

  it("renders the Skills page with its own header", async () => {
    const { default: App } = await import("@/App");
    await renderAppReady(App);
    fireEvent.click(sidebarApp("Skills"));

    expect(await screen.findByTestId("unified-skills-panel")).toHaveTextContent(
      "installed",
    );
  });

  it("navigates OpenClaw and Hermes pages with underline tabs", async () => {
    const { default: App } = await import("@/App");
    await renderAppReady(App);

    fireEvent.click(sidebarApp("OpenClaw"));
    // 首次启动提示是模态对话框，页面其余部分是 aria-hidden，所以带上 hidden
    const openclawTabs = await screen.findByRole("tablist", {
      name: "OpenClaw",
      hidden: true,
    });
    const pageTabs = within(openclawTabs).getAllByRole("tab", { hidden: true });
    expect(pageTabs.map((tab) => tab.textContent)).toEqual([
      "appPage.client",
      "appPage.workspace",
      "appPage.openclawConfig",
    ]);
    expect(pageTabs[0]).toHaveAttribute("aria-selected", "true");
    expect(
      screen.queryByRole("group", { name: "OpenClaw", hidden: true }),
    ).not.toBeInTheDocument();

    fireEvent.click(pageTabs[2]);
    const configTabs = await screen.findByRole("tablist", {
      name: "appPage.openclawConfig",
      hidden: true,
    });
    const subTabs = within(configTabs).getAllByRole("tab", { hidden: true });
    expect(subTabs).toHaveLength(3);
    expect(subTabs[0]).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tabpanel", { hidden: true })).toHaveAttribute(
      "aria-labelledby",
      "openclaw-config-env",
    );

    fireEvent.click(sidebarApp("Hermes"));
    const hermesTabs = await screen.findByRole("tablist", {
      name: "Hermes",
      hidden: true,
    });
    const hermesPageTabs = within(hermesTabs).getAllByRole("tab", {
      hidden: true,
    });
    expect(hermesPageTabs.map((tab) => tab.textContent)).toEqual([
      "appPage.client",
      "appPage.memory",
    ]);

    // 记忆页：页头的 solid 主操作换成「保存」，两份记忆用二级页签
    fireEvent.click(hermesPageTabs[1]);
    const header = document.querySelector("header") as HTMLElement;
    const save = await within(header).findByRole("button", {
      name: "common.save",
      hidden: true,
    });
    expect(save).toHaveAttribute("aria-disabled", "true");
    expect(
      within(header).queryByRole("button", {
        name: /provider.addProvider/,
        hidden: true,
      }),
    ).not.toBeInTheDocument();
    const memoryTabs = await screen.findByRole("tablist", {
      name: "hermes.memory.fileTabs",
      hidden: true,
    });
    expect(
      within(memoryTabs).getAllByRole("tab", { hidden: true }),
    ).toHaveLength(2);
    await waitFor(() =>
      expect(
        screen.getAllByRole("textbox", {
          name: "hermes.memory.editorLabel",
          hidden: true,
        })[0],
      ).toHaveValue("agent notes"),
    );
  });

  // F-02：存过什么旧视图都不恢复，冷启动恒进用量页（含已删除的供应商页、
  // 授权中心和旧的 skillsDiscovery）
  it.each([
    "providers",
    "auth",
    "skillsDiscovery",
    "skills",
    "mcp",
    "settings",
  ])(
    "cold-starts on the usage page whatever view was stored (%s)",
    async (stored) => {
      localStorage.setItem("aicli-client-last-view", stored);
      const { default: App } = await import("@/App");
      await renderAppReady(App);

      await waitFor(() =>
        expect(sidebarApp("nav.usage")).toHaveAttribute("aria-current", "page"),
      );
      expect(screen.queryByTestId("provider-list")).not.toBeInTheDocument();
      expect(
        screen.queryByTestId("unified-skills-panel"),
      ).not.toBeInTheDocument();
      expect(screen.queryByTestId("settings-page")).not.toBeInTheDocument();
    },
  );

  // F-07：未绑定时只出现绑定对话框，主界面（侧栏、页面）不渲染
  it("shows only the bind dialog while the account is not bound", async () => {
    server.use(
      http.post("http://tauri.local/aicli_get_account", () =>
        HttpResponse.json({ ...boundAccount, state: "not_bound" }),
      ),
    );
    const { default: App } = await import("@/App");
    renderApp(App);
    expect(
      await screen.findByRole("dialog", { name: "aicli.bind.gateTitle" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("navigation")).not.toBeInTheDocument();
    expect(screen.queryByTestId("aicli-home")).not.toBeInTheDocument();
  });

  it("does not persist the view while switching pages", async () => {
    const { default: App } = await import("@/App");
    await renderAppReady(App);
    fireEvent.click(sidebarApp("Skills"));
    expect(
      await screen.findByTestId("unified-skills-panel"),
    ).toBeInTheDocument();
    enterClientPage("Claude Code");
    expect(await screen.findByTestId("aicli-client-page")).toBeInTheDocument();
    expect(localStorage.getItem("aicli-client-last-view")).toBeNull();
  });

  // F-04：客户端页取代了供应商列表，页头不能再有供应商搜索和「添加供应商」
  it.each(["Claude Code", "Codex"])(
    "has no provider search or add button on the %s client page",
    async (app) => {
      const { default: App } = await import("@/App");
      await renderAppReady(App);
      enterClientPage(app);
      expect(
        await screen.findByTestId("aicli-client-page"),
      ).toBeInTheDocument();
      const header = document.querySelector("header") as HTMLElement;
      expect(
        within(header).queryByRole("button", {
          name: /provider\.addProvider/,
          hidden: true,
        }),
      ).not.toBeInTheDocument();
      expect(
        within(header).queryByRole("button", {
          name: /provider\.searchAriaLabel/,
          hidden: true,
        }),
      ).not.toBeInTheDocument();
    },
  );

  it("shows all eight native resize cursors and restores existing inline styles", async () => {
    windowMocks.isLinux.mockReturnValue(true);
    setSettings({ useAppWindowControls: true });
    const { default: App } = await import("@/App");
    const { unmount } = await renderAppReady(App);
    const header = document.querySelector("header")!;
    header.style.setProperty("cursor", "help", "important");
    const x = window.innerWidth;
    const y = window.innerHeight;

    await waitFor(() => {
      fireEvent.mouseMove(header, { clientX: x / 2, clientY: 1 });
      expect(header.style.cursor).toBe("ns-resize");
    });
    for (const [clientX, clientY, cursor] of [
      [x / 2, y - 1, "ns-resize"],
      [1, y / 2, "ew-resize"],
      [x - 1, y / 2, "ew-resize"],
      [1, 1, "nwse-resize"],
      [x - 1, y - 1, "nwse-resize"],
      [x - 1, 1, "nesw-resize"],
      [1, y - 1, "nesw-resize"],
    ] as const) {
      fireEvent.mouseMove(header, { clientX, clientY });
      expect(header.style.cursor).toBe(cursor);
      expect(header.style.getPropertyPriority("cursor")).toBe("important");
    }
    fireEvent.mouseMove(header, { clientX: x / 2, clientY: 30 });
    expect(header.style.cursor).toBe("help");
    expect(header.style.getPropertyPriority("cursor")).toBe("important");

    const icon = header.querySelector("svg")!;
    fireEvent.mouseMove(header, { clientX: x / 2, clientY: 1 });
    fireEvent.mouseMove(icon, { clientX: x / 2, clientY: 1 });
    expect(header.style.cursor).toBe("help");
    expect(icon.style.cursor).toBe("ns-resize");
    fireEvent.blur(window);
    expect(icon.style.cursor).toBe("");
    vi.stubGlobal("devicePixelRatio", 2);
    windowMocks.scaleFactor.mockResolvedValue(2);
    try {
      await act(async () => {
        windowMocks.resizeListeners.forEach((handler) => handler());
      });
      fireEvent.mouseMove(header, { clientX: x / 2, clientY: 9 });
      expect(header.style.cursor).toBe("ns-resize");
      fireEvent.mouseMove(header, { clientX: x / 2, clientY: 10 });
      expect(header.style.cursor).toBe("help");
      fireEvent.mouseMove(header, { clientX: x / 2, clientY: 1 });
      unmount();
    } finally {
      vi.unstubAllGlobals();
    }
    expect(header.style.cursor).toBe("help");
  });

  it("lets the top edge resize before Tauri dragging without blocking inner-header or button events", async () => {
    windowMocks.isLinux.mockReturnValue(true);
    setSettings({ useAppWindowControls: true });
    const { default: App } = await import("@/App");
    await renderAppReady(App);
    const header = document.querySelector("header")!;
    await waitFor(() => {
      fireEvent.mouseMove(header, { clientX: 500, clientY: 1 });
      expect(header.style.cursor).toBe("ns-resize");
    });

    // Tauri's drag-region script listens during document bubbling.
    const startDragging = vi.fn();
    const documentMouseDown = vi.fn((event: MouseEvent) => {
      const drag = (event.target as Element).getAttribute(
        "data-tauri-drag-region",
      );
      if (
        drag !== null &&
        drag !== "false" &&
        event.button === 0 &&
        (event.detail === 1 || event.detail === 2)
      ) {
        startDragging();
      }
    });
    document.addEventListener("mousedown", documentMouseDown);
    try {
      for (const detail of [1, 2]) {
        const edgePress = new MouseEvent("mousedown", {
          bubbles: true,
          cancelable: true,
          button: 0,
          detail,
          clientX: 500,
          clientY: 1,
        });
        header.dispatchEvent(edgePress);
        expect(edgePress.defaultPrevented).toBe(false);
      }
      expect(documentMouseDown).not.toHaveBeenCalled();
      expect(startDragging).not.toHaveBeenCalled();
      fireEvent.mouseDown(header, {
        button: 0,
        detail: 1,
        clientX: 500,
        clientY: 30,
      });
      fireEvent.mouseDown(header, { button: 2, clientX: 500, clientY: 1 });
      fireEvent.mouseDown(header.querySelector("button")!, {
        button: 0,
        clientX: 500,
        clientY: 1,
      });
      expect(documentMouseDown).toHaveBeenCalledTimes(3);
      expect(startDragging).toHaveBeenCalledTimes(1);
    } finally {
      document.removeEventListener("mousedown", documentMouseDown);
    }
  });

  it("clears resize hints and leaves events alone when native resizing is unavailable", async () => {
    windowMocks.isLinux.mockReturnValue(true);
    setSettings({ useAppWindowControls: true });
    const { default: App } = await import("@/App");
    await renderAppReady(App);
    const header = document.querySelector("header")!;
    const moveToEdge = () =>
      fireEvent.mouseMove(header, { clientX: 500, clientY: 1 });
    await waitFor(() => {
      moveToEdge();
      expect(header.style.cursor).toBe("ns-resize");
    });
    const documentMouseDown = vi.fn();
    document.addEventListener("mousedown", documentMouseDown);
    try {
      for (const [method, disabledValue] of [
        [windowMocks.isMaximized, true],
        [windowMocks.isFullscreen, true],
        [windowMocks.isResizable, false],
      ] as const) {
        method.mockResolvedValue(disabledValue);
        await act(async () => {
          windowMocks.resizeListeners.forEach((handler) => handler());
        });
        moveToEdge();
        expect(header.style.cursor).toBe("");
        fireEvent.mouseDown(header, { button: 0, clientX: 500, clientY: 1 });
        method.mockResolvedValue(!disabledValue);
        await act(async () => {
          windowMocks.resizeListeners.forEach((handler) => handler());
        });
        moveToEdge();
        expect(header.style.cursor).toBe("ns-resize");
      }
      expect(documentMouseDown).toHaveBeenCalledTimes(3);
    } finally {
      document.removeEventListener("mousedown", documentMouseDown);
    }
  });
});
