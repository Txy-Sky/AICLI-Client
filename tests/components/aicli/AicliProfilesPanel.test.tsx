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
    ...patch,
  };
  invokeMock.mockImplementation(async (command: string, args?: unknown) => {
    const reply = responses[command];
    if (typeof reply === "function")
      return (reply as (a: unknown) => unknown)(args);
    return reply;
  });
}

// 次要操作收在每张卡片的「更多」菜单里；Radix 菜单在 jsdom 里用回车打开最稳
async function openMenu(card: HTMLElement) {
  fireEvent.keyDown(
    within(card).getByRole("button", { name: "aicli.profiles.moreActions" }),
    { key: "Enter" },
  );
  return screen.findByRole("menu");
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
    const menu = await openMenu(
      await screen.findByTestId("aicli-preset-recommended"),
    );
    fireEvent.click(
      within(menu).getByRole("menuitem", { name: "aicli.profiles.viewDiff" }),
    );
    expect(
      await screen.findByTestId("aicli-diff-only-preset"),
    ).toHaveTextContent("shared");
    expect(screen.getByTestId("aicli-diff-only-other")).toHaveTextContent("—");
  });

  it("sets a user profile as default and for one client", async () => {
    respond({ aicli_set_active_profile: undefined });
    open();
    const row = await screen.findByTestId("aicli-profile-mine");
    fireEvent.click(
      within(await openMenu(row)).getByRole("menuitem", {
        name: "aicli.profiles.setDefault",
      }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("aicli_set_active_profile", {
        scope: "global",
        scopeKey: "",
        id: "mine",
      }),
    );
    await waitFor(() =>
      expect(screen.queryByRole("menu")).not.toBeInTheDocument(),
    );
    fireEvent.click(
      within(await openMenu(row)).getByRole("menuitemcheckbox", {
        name: "Codex",
      }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("aicli_set_active_profile", {
        scope: "app",
        scopeKey: "codex",
        id: "mine",
      }),
    );
  });

  it("deletes a user profile only after confirming", async () => {
    respond({ aicli_delete_profile: undefined });
    open();
    const row = await screen.findByTestId("aicli-profile-mine");
    fireEvent.click(
      within(await openMenu(row)).getByRole("menuitem", {
        name: "aicli.profiles.delete",
      }),
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
  it("cannot save with unresolved conflicts; confirming all in group order enables save", async () => {
    respond({
      aicli_save_profile: (args: unknown) =>
        (args as { profile: AicliProfile }).profile,
    });
    open();
    const editor = await editFromPreset();
    expect(within(editor).getByTestId("aicli-preview-shared")).toHaveAttribute(
      "data-status",
      "conflict",
    );
    expect(
      within(editor).getByText("aicli.profiles.pendingConflicts"),
    ).toBeInTheDocument();
    const save = within(editor).getByRole("button", {
      name: "aicli.profiles.save",
    });
    expect(save).toBeDisabled();

    fireEvent.click(
      within(editor).getByRole("button", { name: "aicli.profiles.confirmAll" }),
    );
    expect(within(editor).getByTestId("aicli-preview-shared")).toHaveAttribute(
      "data-status",
      "confirmed",
    );
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
