import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AicliProfilesPanel } from "@/components/aicli/profiles/AicliProfilesPanel";
import { PROFILE_ERROR_KEYS } from "@/components/aicli/profiles/AicliProfileEditor";
import type {
  AicliCatalog,
  AicliProfile,
  AicliProfilesView,
  AicliRoutesView,
} from "@/types/aicli";
import { renderWithQueryClient } from "../../utils/testQueryClient";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

const catalog: AicliCatalog = {
  groups: {
    cheap: { description: "", ratio: 0.5, ratio_known: true },
    mid: { description: "", ratio: 1, ratio_known: true },
  },
  auto_available: false,
  auto_groups: [],
  models: {
    shared: {
      groups: ["cheap", "mid"],
      wire: { openai: "openai" },
      compat: {},
      pricing: {},
    },
    unique: {
      groups: ["mid"],
      wire: { anthropic: "anthropic" },
      compat: {},
      pricing: {},
    },
  },
};

// 后端预设的 name 就是 id（profiles.rs::preset），显示名由前端翻译（F-11）
const preset: AicliProfile = {
  id: "recommended",
  name: "recommended",
  source: "system",
  preset_id: "recommended",
  revision: 0,
  body: {
    selected_groups: ["mid", "cheap"],
    selection_mode: "fixed_priority",
    model_bindings: [],
    resolved_conflicts: [],
    line_policy: "inherit",
    allow_fallback: true,
    excluded_models: [],
  },
};

const mine: AicliProfile = {
  ...preset,
  id: "mine",
  name: "我的",
  source: "user",
  preset_id: null,
  revision: 4,
  body: { ...preset.body, resolved_conflicts: ["shared"] },
};

const view = (patch: Partial<AicliProfilesView> = {}): AicliProfilesView => ({
  profiles: [mine],
  presets: [preset],
  bindings: [],
  active_profile_id: "recommended",
  ...patch,
});

const routes = (repair = false): AicliRoutesView => ({
  fingerprint: "f",
  active_profile_id: "recommended",
  selected_line_id: "main",
  profiles: [
    {
      id: "recommended",
      name: "recommended",
      source: "system",
      preset_id: "recommended",
      revision: 0,
      routes: [
        {
          public_model: "shared",
          upstream_model: "shared",
          ordered_groups: ["mid"],
        },
        {
          public_model: "unique",
          upstream_model: "unique",
          ordered_groups: ["mid"],
        },
      ],
      repair: {
        conservative: false,
        needs_repair: false,
        missing_groups: [],
        affected_models: [],
      },
    },
    {
      id: "mine",
      name: "我的",
      source: "user",
      preset_id: null,
      revision: 4,
      routes: [
        {
          public_model: "unique",
          upstream_model: "unique",
          ordered_groups: ["mid"],
        },
      ],
      repair: {
        conservative: false,
        needs_repair: repair,
        missing_groups: repair ? ["vip"] : [],
        affected_models: repair ? ["unique"] : [],
      },
    },
  ],
});

function respond(patch: Record<string, unknown> = {}) {
  const responses: Record<string, unknown> = {
    aicli_get_profiles: view(),
    aicli_get_routes: routes(),
    aicli_get_catalog: catalog,
    aicli_takeover_status: [],
    ...patch,
  };
  invokeMock.mockImplementation(async (command: string, args?: unknown) => {
    const reply = responses[command];
    if (typeof reply === "function")
      return (reply as (a: unknown) => unknown)(args);
    return reply;
  });
}

const open = () =>
  renderWithQueryClient(
    <AicliProfilesPanel isOpen onClose={() => undefined} />,
  );

async function editFromPreset() {
  fireEvent.click(
    within(await screen.findByTestId("aicli-preset-recommended")).getByRole(
      "button",
      {
        name: "aicli.profiles.newFromPreset",
      },
    ),
  );
  return screen.findByTestId("aicli-profile-editor");
}

beforeEach(() => {
  invokeMock.mockReset();
});

describe("AicliProfilesPanel — list", () => {
  it("says profiles stay on this device and shows presets read only", async () => {
    respond();
    open();
    expect(
      await screen.findByText("aicli.profiles.localOnly"),
    ).toBeInTheDocument();
    const presetRow = await screen.findByTestId("aicli-preset-recommended");
    expect(
      within(presetRow).getByText("aicli.profiles.readOnly"),
    ).toBeInTheDocument();
    expect(
      within(presetRow).queryByRole("button", { name: "aicli.profiles.edit" }),
    ).not.toBeInTheDocument();
    expect(
      within(presetRow).queryByRole("button", {
        name: "aicli.profiles.delete",
      }),
    ).not.toBeInTheDocument();
  });

  it("shows system presets by their translated names, not their ids", async () => {
    respond();
    open();
    const presetRow = await screen.findByTestId("aicli-preset-recommended");
    expect(
      within(presetRow).getByRole("heading", {
        name: "aicli.profiles.presets.recommended.name",
      }),
    ).toBeInTheDocument();
    expect(
      within(presetRow).getByText("aicli.profiles.presets.recommended.desc"),
    ).toBeInTheDocument();
    expect(
      within(presetRow).queryByText("recommended"),
    ).not.toBeInTheDocument();
    // 我的配置集用自己的名字，没有预设说明
    const mineRow = screen.getByTestId("aicli-profile-mine");
    expect(
      within(mineRow).getByRole("heading", { name: "我的" }),
    ).toBeInTheDocument();
  });

  it("shows the preset's model differences against the default profile", async () => {
    respond({
      aicli_get_profiles: view({
        bindings: [{ scope: "global", scope_key: "", profile_id: "mine" }],
      }),
    });
    open();
    fireEvent.click(
      within(await screen.findByTestId("aicli-preset-recommended")).getByRole(
        "button",
        { name: "aicli.profiles.viewDiff" },
      ),
    );
    expect(
      await screen.findByTestId("aicli-diff-only-preset"),
    ).toHaveTextContent("shared");
    expect(screen.getByTestId("aicli-diff-only-other")).toHaveTextContent("—");
    // 有关闭按钮（点遮罩不关），关得掉
    fireEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", {
        name: "common.close",
      }),
    );
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
  });

  it("offers no diff for the preset that is already the default", async () => {
    respond();
    open();
    const presetCard = await screen.findByTestId("aicli-preset-recommended");
    // 默认就是推荐：自己和自己比没有意义；预设也不能删
    expect(
      within(presetCard).queryByRole("button", {
        name: "aicli.profiles.viewDiff",
      }),
    ).not.toBeInTheDocument();
    expect(
      within(presetCard).queryByRole("button", {
        name: "aicli.profiles.delete",
      }),
    ).not.toBeInTheDocument();
  });

  it("sets a user profile as default from its card", async () => {
    respond({ aicli_set_active_profile: undefined });
    open();
    const row = await screen.findByTestId("aicli-profile-mine");
    // 已经是默认的那份不再提供「设为默认」
    expect(
      within(screen.getByTestId("aicli-preset-recommended")).queryByRole(
        "button",
        { name: "aicli.profiles.setDefault" },
      ),
    ).not.toBeInTheDocument();
    fireEvent.click(
      within(row).getByRole("button", { name: "aicli.profiles.setDefault" }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("aicli_set_active_profile", {
        scope: "global",
        scopeKey: "",
        id: "mine",
      }),
    );
  });

  it("picks the global default and a client's own profile under who uses which", async () => {
    respond({
      aicli_set_active_profile: undefined,
      aicli_get_profiles: view({
        bindings: [{ scope: "app", scope_key: "claude", profile_id: "mine" }],
      }),
    });
    open();
    const global = within(
      await screen.findByTestId("aicli-scope-global"),
    ).getByRole("combobox");
    await waitFor(() => expect(global).toHaveValue("recommended"));
    fireEvent.change(global, { target: { value: "mine" } });
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("aicli_set_active_profile", {
        scope: "global",
        scopeKey: "",
        id: "mine",
      }),
    );

    // 没有单独指定的客户端显示「跟随默认」，可以指定一份
    const codex = within(screen.getByTestId("aicli-scope-codex")).getByRole(
      "combobox",
    );
    expect(codex).toHaveValue("");
    await waitFor(() => expect(codex).toBeEnabled());
    fireEvent.change(codex, { target: { value: "mine" } });
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("aicli_set_active_profile", {
        scope: "app",
        scopeKey: "codex",
        id: "mine",
      }),
    );

    // 已单独指定的客户端可以改回跟随默认（清除该作用域）
    const claude = within(screen.getByTestId("aicli-scope-claude")).getByRole(
      "combobox",
    );
    expect(claude).toHaveValue("mine");
    await waitFor(() => expect(claude).toBeEnabled());
    fireEvent.change(claude, { target: { value: "" } });
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("aicli_set_active_profile", {
        scope: "app",
        scopeKey: "claude",
        id: null,
      }),
    );
  });

  it("names the taken-over clients that actually use each profile", async () => {
    respond({
      aicli_takeover_status: [
        { app: "claude", state: "active", taken_over_at: 1 },
        { app: "codex", state: "active", taken_over_at: 1 },
        { app: "gemini", state: "inactive", taken_over_at: null },
      ],
      aicli_get_profiles: view({
        bindings: [
          { scope: "app", scope_key: "codex", profile_id: "mine" },
          { scope: "app", scope_key: "gemini", profile_id: "mine" },
        ],
      }),
    });
    open();
    const presetCard = await screen.findByTestId("aicli-preset-recommended");
    // Claude Code 跟随默认（推荐）；Codex 单独指定；Gemini 指定了但没接管，不算在用
    await within(presetCard).findByText("aicli.profiles.inUse");
    expect(within(presetCard).getByText("Claude Code")).toBeInTheDocument();
    expect(within(presetCard).queryByText("Codex")).not.toBeInTheDocument();
    const mineCard = screen.getByTestId("aicli-profile-mine");
    expect(within(mineCard).getByText("Codex")).toBeInTheDocument();
    expect(within(mineCard).queryByText("Gemini CLI")).not.toBeInTheDocument();
    expect(
      within(screen.getByTestId("aicli-scope-claude")).getByText(
        "aicli.profiles.clientActive",
      ),
    ).toBeInTheDocument();
    expect(
      within(screen.getByTestId("aicli-scope-gemini")).getByText(
        "aicli.profiles.clientInactive",
      ),
    ).toBeInTheDocument();
  });

  it("says so when no client is using a profile", async () => {
    respond();
    open();
    const mineCard = await screen.findByTestId("aicli-profile-mine");
    expect(
      within(mineCard).getByText("aicli.profiles.notInUse"),
    ).toBeInTheDocument();
  });

  it("deletes a user profile only after confirming", async () => {
    respond({ aicli_delete_profile: undefined });
    open();
    const row = await screen.findByTestId("aicli-profile-mine");
    fireEvent.click(
      within(row).getByRole("button", { name: "aicli.profiles.delete" }),
    );
    const dialog = await screen.findByRole("dialog");
    expect(invokeMock).not.toHaveBeenCalledWith(
      "aicli_delete_profile",
      expect.anything(),
    );
    fireEvent.click(
      within(dialog).getByRole("button", { name: "aicli.profiles.delete" }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("aicli_delete_profile", {
        id: "mine",
        revision: 4,
      }),
    );
  });

  it("names the clients that fall back to the default when deleting", async () => {
    respond({
      aicli_get_profiles: view({
        bindings: [{ scope: "app", scope_key: "codex", profile_id: "mine" }],
      }),
    });
    open();
    fireEvent.click(
      within(await screen.findByTestId("aicli-profile-mine")).getByRole(
        "button",
        { name: "aicli.profiles.delete" },
      ),
    );
    const users = within(await screen.findByRole("dialog")).getByTestId(
      "aicli-delete-users",
    );
    expect(within(users).getByText("Codex")).toBeInTheDocument();
    expect(
      within(users).getByText("aicli.profiles.deleteFallback"),
    ).toBeInTheDocument();
  });

  it("says no client is set to it when deleting an unused profile", async () => {
    respond();
    open();
    fireEvent.click(
      within(await screen.findByTestId("aicli-profile-mine")).getByRole(
        "button",
        { name: "aicli.profiles.delete" },
      ),
    );
    const dialog = await screen.findByRole("dialog");
    expect(
      within(dialog).getByText("aicli.profiles.deleteNoUsers"),
    ).toBeInTheDocument();
    expect(
      within(dialog).queryByTestId("aicli-delete-users"),
    ).not.toBeInTheDocument();
  });

  it("lists missing groups and affected models when a profile needs repair", async () => {
    respond({ aicli_get_routes: routes(true) });
    open();
    const row = await screen.findByTestId("aicli-profile-mine");
    expect(
      await within(row).findByText("aicli.profiles.needsRepairTitle"),
    ).toBeInTheDocument();
    expect(
      within(row).getByText("aicli.profiles.needsRepairBody"),
    ).toBeInTheDocument();
  });
});

describe("AicliProfileEditor — conflicts and saving", () => {
  it("a copy of a preset starts with its multi-group models confirmed by group order", async () => {
    respond({
      aicli_save_profile: (args: unknown) =>
        (args as { profile: AicliProfile }).profile,
    });
    open();
    const editor = await editFromPreset();
    // 复制即确认预设的分组顺序（与后端 copy_preset 一致），不再满屏「冲突：未处理」
    expect(within(editor).getByTestId("aicli-preview-shared")).toHaveAttribute(
      "data-status",
      "confirmed",
    );
    expect(
      within(editor).queryByText("aicli.profiles.pendingConflicts"),
    ).not.toBeInTheDocument();
    const save = within(editor).getByRole("button", {
      name: "aicli.profiles.save",
    });
    expect(save).toBeEnabled();
    fireEvent.click(save);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "aicli_save_profile",
        expect.anything(),
      ),
    );
    const saved = invokeMock.mock.calls.find(
      ([c]) => c === "aicli_save_profile",
    )![1] as {
      profile: AicliProfile;
    };
    expect(saved.profile).toMatchObject({
      source: "system_copy",
      preset_id: "recommended",
      revision: 0,
      name: "aicli.profiles.copyName",
    });
    expect(saved.profile.body.resolved_conflicts).toEqual(["shared"]);
  });

  it("resolves a conflict by excluding the model or picking one source", async () => {
    respond();
    open();
    const editor = await editFromPreset();
    const choice = within(editor).getByRole("combobox", {
      name: "aicli.profiles.choiceFor",
    });
    fireEvent.change(choice, { target: { value: "exclude" } });
    expect(within(editor).getByTestId("aicli-preview-shared")).toHaveAttribute(
      "data-status",
      "excluded",
    );
    expect(
      within(editor).getByRole("button", { name: "aicli.profiles.save" }),
    ).toBeEnabled();
    fireEvent.change(choice, { target: { value: "source:cheap" } });
    expect(within(editor).getByTestId("aicli-preview-shared")).toHaveAttribute(
      "data-status",
      "bound",
    );
  });

  it("reorders groups with the move buttons", async () => {
    respond();
    open();
    const editor = await editFromPreset();
    const groups = () =>
      within(editor)
        .getAllByTestId(/^aicli-group-/)
        .map((item) => item.getAttribute("data-testid"));
    expect(groups()).toEqual(["aicli-group-mid", "aicli-group-cheap"]);
    fireEvent.click(
      within(within(editor).getByTestId("aicli-group-cheap")).getByRole(
        "button",
        {
          name: "aicli.profiles.moveUp",
        },
      ),
    );
    expect(groups()).toEqual(["aicli-group-cheap", "aicli-group-mid"]);
  });

  it("shows where models land and moves a cheaper group ahead in one click", async () => {
    respond();
    open();
    const editor = await editFromPreset();
    // 顺序 mid（×1）→ cheap（×0.5）：两个模型都落在 mid，其中 shared 在 cheap 更便宜
    const mid = within(editor).getByTestId("aicli-landing-mid");
    expect(
      within(mid).getByText("aicli.profiles.landingCheaper"),
    ).toBeInTheDocument();
    expect(
      within(editor).queryByTestId("aicli-landing-cheap"),
    ).not.toBeInTheDocument();
    fireEvent.click(
      within(mid).getByRole("button", { name: "aicli.profiles.landingMoveUp" }),
    );
    expect(
      within(editor)
        .getAllByTestId(/^aicli-group-/)
        .map((item) => item.getAttribute("data-testid")),
    ).toEqual(["aicli-group-cheap", "aicli-group-mid"]);
    expect(
      within(editor).getByTestId("aicli-landing-cheap"),
    ).toBeInTheDocument();
    expect(
      within(editor).queryByText("aicli.profiles.landingCheaper"),
    ).not.toBeInTheDocument();
  });

  it.each(Object.entries(PROFILE_ERROR_KEYS))(
    "shows a message for the %s validation error",
    async (code, key) => {
      respond({
        aicli_save_profile: () => {
          throw { code, message: code };
        },
      });
      open();
      const editor = await editFromPreset();
      fireEvent.click(
        within(editor).getByRole("button", {
          name: "aicli.profiles.confirmAll",
        }),
      );
      fireEvent.click(
        within(editor).getByRole("button", { name: "aicli.profiles.save" }),
      );
      expect(
        await within(editor).findByText(`aicli.profiles.errors.${key}`),
      ).toBeInTheDocument();
    },
  );
});
