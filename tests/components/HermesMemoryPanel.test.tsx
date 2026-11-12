import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  limits: {
    memory: 2200,
    user: 1375,
    memoryEnabled: true,
    userEnabled: true,
  },
  save: vi.fn(),
  toggle: vi.fn(),
  openWebUI: vi.fn(),
  toastSuccess: vi.fn(),
}));

vi.mock("sonner", () => ({
  toast: { success: mocks.toastSuccess, error: vi.fn() },
}));

vi.mock("@/hooks/useHermes", () => ({
  useHermesMemory: (kind: string) => ({
    data: kind === "memory" ? "agent notes" : "user profile",
    isLoading: false,
    isError: false,
  }),
  useHermesMemoryLimits: () => ({ data: mocks.limits }),
  useHermesConfigDir: () => ({ data: "/Users/octocat/.hermes" }),
  useOpenHermesWebUI: () => mocks.openWebUI,
  useSaveHermesMemory: () => ({ mutateAsync: mocks.save, isPending: false }),
  useToggleHermesMemoryEnabled: () => ({
    mutate: mocks.toggle,
    isPending: false,
  }),
}));

import HermesMemoryPanel from "@/components/hermes/HermesMemoryPanel";

function renderPage(onOpenWebUI?: (path?: string) => void) {
  return render(<HermesMemoryPanel onOpenWebUI={onOpenWebUI} />);
}

const pane = (kind: "memory" | "user") =>
  screen.getByTestId(`hermes-memory-${kind}`);

const editors = () =>
  screen.getAllByLabelText("hermes.memory.editorLabel", {
    selector: "textarea",
  });

describe("HermesMemoryPanel", () => {
  beforeEach(() => {
    mocks.limits = {
      memory: 2200,
      user: 1375,
      memoryEnabled: true,
      userEnabled: true,
    };
    mocks.save.mockReset().mockResolvedValue(undefined);
    mocks.toggle.mockReset();
    mocks.openWebUI.mockReset();
    mocks.toastSuccess.mockReset();
  });

  it("shows both memory files side by side and marks unsaved edits", () => {
    renderPage();

    expect(screen.queryByRole("tab")).not.toBeInTheDocument();
    expect(
      screen.getAllByRole("heading").map((heading) => heading.textContent),
    ).toEqual([
      "hermes.memory.agentNameMEMORY.md",
      "hermes.memory.userNameUSER.md",
    ]);
    expect(editors()).toHaveLength(2);
    expect(
      screen.queryByRole("img", { name: "hermes.memory.unsaved" }),
    ).not.toBeInTheDocument();

    fireEvent.change(editors()[0], { target: { value: "draft" } });
    // 有未保存修改的那份标题旁出现小圆点
    expect(within(pane("memory")).getByRole("heading")).toContainElement(
      screen.getByRole("img", { name: "hermes.memory.unsaved" }),
    );
    expect(
      within(pane("user")).queryByRole("img", {
        name: "hermes.memory.unsaved",
      }),
    ).not.toBeInTheDocument();
  });

  it("saves each file with its own button and clears the dot", async () => {
    renderPage();

    const userSave = within(pane("user")).getByRole("button", {
      name: "hermes.memory.saveOne",
    });
    expect(userSave).toHaveAttribute("aria-disabled", "true");
    fireEvent.click(userSave);
    expect(mocks.save).not.toHaveBeenCalled();

    fireEvent.change(editors()[1], { target: { value: "new profile" } });
    const enabledSave = within(pane("user")).getByRole("button", {
      name: "hermes.memory.saveOne",
    });
    expect(enabledSave).not.toHaveAttribute("aria-disabled");
    // 另一份没改，它的保存仍不可用
    expect(
      within(pane("memory")).getByRole("button", {
        name: "hermes.memory.saveOne",
      }),
    ).toHaveAttribute("aria-disabled", "true");
    fireEvent.click(enabledSave);

    await waitFor(() =>
      expect(mocks.save).toHaveBeenCalledWith({
        kind: "user",
        content: "new profile",
      }),
    );
    // 「下次启动或新建会话时生效」是后果，写在保存成功的 toast 里
    await waitFor(() =>
      expect(mocks.toastSuccess).toHaveBeenCalledWith(
        "hermes.memory.saveSuccess",
      ),
    );
    expect(
      screen.queryByRole("img", { name: "hermes.memory.unsaved" }),
    ).not.toBeInTheDocument();
  });

  it("shows the file path when enabled and the skip warning when disabled", () => {
    mocks.limits = { ...mocks.limits, userEnabled: false };
    renderPage();

    const agentPanel = pane("memory");
    const userPanel = pane("user");
    expect(agentPanel).toHaveTextContent("~/.hermes/memories/MEMORY.md");
    expect(userPanel).toHaveTextContent("hermes.memory.disabledHint");
    // 标题里常驻文件名；禁用时不写完整路径
    expect(userPanel).not.toHaveTextContent("~/.hermes/memories/USER.md");

    const switches = screen.getAllByRole("switch", { hidden: true });
    fireEvent.click(switches[1]);
    expect(mocks.toggle).toHaveBeenCalledWith(
      { kind: "user", enabled: true },
      expect.objectContaining({ onSuccess: expect.any(Function) }),
    );
    mocks.toggle.mock.calls[0][1].onSuccess();
    expect(mocks.toastSuccess).toHaveBeenCalledWith("hermes.memory.toggledOn");
  });

  it("writes the budget as remaining characters and flags overflow", () => {
    mocks.limits = { ...mocks.limits, memory: 10 };
    renderPage();

    const meter = screen.getAllByRole("meter")[0];
    expect(meter).toHaveAttribute("aria-valuenow", "0");
    expect(pane("memory")).toHaveTextContent("hermes.memory.overLimitBy");

    fireEvent.change(editors()[0], { target: { value: "short" } });
    expect(screen.getAllByRole("meter")[0]).toHaveAttribute(
      "aria-valuenow",
      "5",
    );
    expect(pane("memory")).toHaveTextContent("hermes.memory.remaining");
  });

  it("opens the Hermes Web UI config page through the caller's handler", () => {
    const onOpen = vi.fn();
    renderPage(onOpen);

    fireEvent.click(
      screen.getByRole("button", { name: /hermes.memory.openConfig/ }),
    );
    expect(onOpen).toHaveBeenCalledWith("/config");
    expect(mocks.openWebUI).not.toHaveBeenCalled();
  });
});
