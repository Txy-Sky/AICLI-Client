import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { McpDiscover } from "@/components/mcp/McpDiscover";
import type { McpCatalogEntry } from "@/lib/mcpRegistry";
import type { McpServer } from "@/types";

const m = vi.hoisted(() => ({
  upsert: vi.fn(),
  registry: {
    data: undefined as McpCatalogEntry[] | undefined,
    isFetching: false,
    isError: false,
    error: null as unknown,
    refetch: vi.fn(),
  },
  registryQueries: [] as string[],
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock("@/hooks/useMcp", () => ({
  useUpsertMcpServer: () => ({ mutateAsync: m.upsert }),
  useSearchMcpRegistry: (query: string) => {
    m.registryQueries.push(query);
    return query.length >= 2 ? m.registry : { ...m.registry, data: [] };
  },
}));

// jsdom 没有布局：虚拟网格按一列、全部渲染
vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({ count }: { count: number }) => ({
    getTotalSize: () => count * 156,
    getVirtualItems: () =>
      Array.from({ length: count }, (_, index) => ({
        index,
        key: index,
        start: index * 156,
      })),
  }),
}));

vi.mock("sonner", () => ({
  toast: {
    success: m.toastSuccess,
    error: m.toastError,
    info: vi.fn(),
    warning: vi.fn(),
  },
}));

function renderDiscover(existing: Record<string, McpServer> = {}) {
  const onShowInstalled = vi.fn();
  render(
    <McpDiscover
      renderViewTabs={(trailing) => trailing}
      existingServers={existing}
      visibleAppIds={["claude", "codex", "pi"]}
      onShowInstalled={onShowInstalled}
    />,
  );
  return { onShowInstalled };
}

const list = () =>
  screen.getByRole("list", { name: "mcpPage.discover.listLabel" });
const itemNamed = (name: string) =>
  within(list())
    .getAllByRole("listitem")
    .find((item) => within(item).queryAllByText(name).length > 0)!;

describe("McpDiscover", () => {
  beforeEach(() => {
    m.upsert.mockReset().mockResolvedValue(undefined);
    m.registry.data = undefined;
    m.registry.isError = false;
    m.registry.error = null;
    m.registryQueries = [];
  });

  it("shows featured servers as cards by default and remembers the list view", async () => {
    const user = userEvent.setup();
    renderDiscover();
    expect(itemNamed("Context7")).toBeTruthy();
    expect(itemNamed("GitHub")).toBeTruthy();
    // 卡片里有 id 和运行方式
    expect(within(itemNamed("Playwright")).getByText("npx")).toBeTruthy();
    expect(
      within(itemNamed("Playwright")).getByText("playwright"),
    ).toBeTruthy();

    const listButton = screen.getByRole("button", {
      name: "discoverView.list",
    });
    expect(listButton.getAttribute("aria-pressed")).toBe("false");
    await user.click(listButton);
    expect(listButton.getAttribute("aria-pressed")).toBe("true");
    expect(localStorage.getItem("cc-switch.mcp.discoverView")).toBe("list");
    expect(itemNamed("Context7")).toBeTruthy();
  });

  it("adds a server without required inputs in one click", async () => {
    const user = userEvent.setup();
    renderDiscover();
    await user.click(
      within(itemNamed("DeepWiki")).getByRole("button", {
        name: "mcpPage.discover.addAria",
      }),
    );
    expect(m.upsert).toHaveBeenCalledTimes(1);
    const server = m.upsert.mock.calls[0][0] as McpServer;
    expect(server.id).toBe("deepwiki");
    expect(server.server).toEqual({
      type: "http",
      url: "https://mcp.deepwiki.com/mcp",
    });
    expect(server.apps).toMatchObject({ claude: true, codex: true, pi: true });
    expect(m.toastSuccess).toHaveBeenCalled();
  });

  it("asks for a required key before adding and writes it as a header", async () => {
    const user = userEvent.setup();
    renderDiscover();
    const addButton = within(itemNamed("GitHub")).getByRole("button", {
      name: "mcpPage.discover.addAria",
    });
    expect(addButton.textContent).toBe("mcpPage.discover.addEllipsis");
    await user.click(addButton);

    const dialog = await screen.findByRole("dialog");
    const submit = within(dialog).getByRole("button", {
      name: "mcpPage.discover.add",
    });
    await user.click(submit);
    expect(m.upsert).not.toHaveBeenCalled();
    expect(
      within(dialog).getByText("mcpPage.discover.fillRequired"),
    ).toBeTruthy();

    const field = within(dialog).getByLabelText(
      "mcpPage.discover.labels.token",
    );
    expect(field.getAttribute("type")).toBe("password");
    await user.type(field, "github_pat_x");
    // 不写入 Codex
    await user.click(within(dialog).getByRole("checkbox", { name: /Codex/ }));
    await user.click(submit);

    expect(m.upsert).toHaveBeenCalledTimes(1);
    const server = m.upsert.mock.calls[0][0] as McpServer;
    expect(server.server.headers).toEqual({
      Authorization: "Bearer github_pat_x",
    });
    expect(server.apps).toMatchObject({ claude: true, codex: false });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("marks servers already in the list and links to them", async () => {
    const user = userEvent.setup();
    const { onShowInstalled } = renderDiscover({
      "my-wiki": {
        id: "my-wiki",
        name: "my-wiki",
        server: { type: "http", url: "https://mcp.deepwiki.com/mcp/" },
        apps: {
          claude: true,
          codex: false,
          gemini: false,
          opencode: false,
          openclaw: false,
          hermes: false,
        },
      },
    });
    const added = within(itemNamed("DeepWiki")).getByRole("button", {
      name: "mcpPage.discover.addedAria",
    });
    await user.click(added);
    expect(onShowInstalled).toHaveBeenCalledWith("my-wiki");
  });

  it("merges registry results and keeps featured matches when the registry fails", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
      m.registry.data = [
        {
          id: "postgres",
          title: "Postgres",
          publisher: "someone",
          description: "SQL access",
          spec: { type: "http", url: "https://pg.example/mcp" },
          inputs: [],
          source: "registry",
          registryName: "io.github.someone/postgres",
        },
      ];
      renderDiscover();
      await user.type(
        screen.getByRole("textbox", { name: "mcpPage.discover.searchAria" }),
        "postgres",
      );
      await act(async () => {
        vi.advanceTimersByTime(400);
      });
      expect(m.registryQueries).toContain("postgres");
      expect(itemNamed("Postgres")).toBeTruthy();
    } finally {
      vi.useRealTimers();
    }
  });

  it("shows a warning with retry when the registry search fails", async () => {
    m.registry.isError = true;
    m.registry.error = new Error("timeout");
    const user = userEvent.setup();
    renderDiscover();
    await user.type(
      screen.getByRole("textbox", { name: "mcpPage.discover.searchAria" }),
      "github{Enter}",
    );
    expect(
      await screen.findByText("mcpPage.discover.registryFailedTitle"),
    ).toBeTruthy();
    // 精选里匹配的照常显示
    expect(itemNamed("GitHub")).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "common.retry" }));
    expect(m.registry.refetch).toHaveBeenCalled();
  });
});
