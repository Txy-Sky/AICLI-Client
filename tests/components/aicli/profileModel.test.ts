import { describe, expect, it } from "vitest";
import {
  applyAlias,
  applyChoice,
  confirmAllByGroupOrder,
  copyPreset,
  diffModelLists,
  moveGroup,
  newProfileId,
  normalizeForSave,
  previewModels,
  toggleGroup,
  unresolvedConflicts,
} from "@/components/aicli/profiles/profileModel";
import type { AicliCatalog, AicliProfileBody } from "@/types/aicli";

const catalog: AicliCatalog = {
  groups: {
    cheap: { description: "", ratio: 0.5, ratio_known: true },
    mid: { description: "", ratio: 1, ratio_known: true },
    vip: { description: "", ratio: null, ratio_known: false },
  },
  auto_available: false,
  auto_groups: [],
  models: {
    shared: {
      groups: ["cheap", "mid"],
      wire: { openai: "openai" },
      compat: {},
      pricing: { tags: "tools,vision" },
    },
    unique: {
      groups: ["mid"],
      wire: { anthropic: "anthropic" },
      compat: {},
      pricing: {},
    },
    elsewhere: { groups: ["vip"], wire: {}, compat: {}, pricing: {} },
  },
};

const body = (patch: Partial<AicliProfileBody> = {}): AicliProfileBody => ({
  selected_groups: ["mid", "cheap"],
  selection_mode: "fixed_priority",
  model_bindings: [],
  resolved_conflicts: [],
  line_policy: "inherit",
  allow_fallback: true,
  excluded_models: [],
  ...patch,
});

describe("profile model (mirrors backend validate)", () => {
  it("previews models from selected groups in group order and flags conflicts", () => {
    const rows = previewModels(body(), catalog);
    expect(rows.map((row) => row.model)).toEqual(["shared", "unique"]);
    const shared = rows.find((row) => row.model === "shared")!;
    expect(shared.sourceGroups).toEqual(["mid", "cheap"]);
    expect(shared.status).toBe("conflict");
    expect(shared.tags).toEqual(["tools", "vision"]);
    expect(rows.find((row) => row.model === "unique")!.status).toBe("single");
    expect(unresolvedConflicts(body(), catalog)).toEqual(["shared"]);
  });

  it("resolves conflicts by confirming, choosing a source, aliasing or excluding", () => {
    expect(
      unresolvedConflicts(confirmAllByGroupOrder(body(), catalog), catalog),
    ).toEqual([]);
    const source = applyChoice(body(), "shared", {
      kind: "source",
      group: "cheap",
    });
    expect(source.model_bindings).toEqual([
      { model: "shared", source_group: "cheap", alias: null },
    ]);
    expect(unresolvedConflicts(source, catalog)).toEqual([]);
    const aliased = applyAlias(body(), "shared", "shared-cheap");
    expect(unresolvedConflicts(aliased, catalog)).toEqual([]);
    expect(previewModels(aliased, catalog)[0].publicName).toBe("shared-cheap");
    const excluded = applyChoice(source, "shared", { kind: "exclude" });
    expect(excluded.excluded_models).toEqual(["shared"]);
    expect(excluded.model_bindings).toEqual([]);
    expect(
      previewModels(excluded, catalog).find((r) => r.model === "shared")!
        .status,
    ).toBe("excluded");
    // 回到按分组顺序：撤掉排除，记为已确认
    const back = applyChoice(excluded, "shared", { kind: "order" });
    expect(back.excluded_models).toEqual([]);
    expect(back.resolved_conflicts).toEqual(["shared"]);
  });

  it("clearing an alias without a source brings the conflict back", () => {
    const aliased = applyAlias(body(), "shared", "x");
    const cleared = applyAlias(aliased, "shared", "  ");
    expect(cleared.model_bindings).toEqual([]);
    expect(unresolvedConflicts(cleared, catalog)).toEqual(["shared"]);
  });

  it("drops source bindings to a group when it is unticked", () => {
    const source = applyChoice(body(), "shared", {
      kind: "source",
      group: "cheap",
    });
    const unticked = toggleGroup(source, "cheap", false);
    expect(unticked.selected_groups).toEqual(["mid"]);
    expect(unticked.model_bindings[0].source_group).toBeNull();
    expect(toggleGroup(body(), "vip", true).selected_groups).toEqual([
      "mid",
      "cheap",
      "vip",
    ]);
  });

  it("normalizes stale records before saving", () => {
    const stale = body({
      selected_groups: ["mid"],
      model_bindings: [
        { model: "elsewhere", source_group: null, alias: "x" },
        { model: "unique", source_group: null, alias: null },
      ],
      resolved_conflicts: ["shared", "gone"],
      excluded_models: ["elsewhere"],
    });
    const clean = normalizeForSave(stale, catalog);
    expect(clean.model_bindings).toEqual([]);
    expect(clean.resolved_conflicts).toEqual(["shared"]);
    expect(clean.excluded_models).toEqual([]);
  });

  it("reorders groups and ignores out-of-range moves", () => {
    expect(moveGroup(body(), 1, 0).selected_groups).toEqual(["cheap", "mid"]);
    expect(moveGroup(body(), 0, -1)).toEqual(body());
  });

  it("copies a preset as a new local profile", () => {
    const preset = {
      id: "recommended",
      name: "推荐",
      source: "system",
      preset_id: "recommended",
      revision: 0,
      body: body(),
    };
    const copy = copyPreset(preset, "user-1", "推荐（副本）");
    expect(copy).toMatchObject({
      id: "user-1",
      name: "推荐（副本）",
      source: "system_copy",
      preset_id: "recommended",
      revision: 0,
    });
    copy.body.selected_groups.push("vip");
    expect(preset.body.selected_groups).toEqual(["mid", "cheap"]);
    expect(newProfileId(["user-a"], 10)).toBe("user-a-1");
    expect(newProfileId([], 10)).toBe("user-a");
  });

  it("diffs model lists by public name", () => {
    expect(diffModelLists(["a", "b"], ["b", "c"])).toEqual({
      onlyInPreset: ["a"],
      onlyInOther: ["c"],
    });
  });
});
