import type {
  AicliCatalog,
  AicliModelBinding,
  AicliProfile,
  AicliProfileBody,
} from "@/types/aicli";

/**
 * 配置集编辑的纯逻辑。冲突判定与后端 `aicli::profiles::validate` 一致：模型在已选
 * 分组中出现多于一次、没有绑定（选来源或起别名）、没有确认、没有排除，就是未解决
 * 的冲突，保存会被后端拒绝（unresolved_conflicts）。
 */

export type AicliPreviewStatus =
  | "single"
  | "conflict"
  | "confirmed"
  | "bound"
  | "excluded";

export interface AicliPreviewRow {
  model: string;
  /** 公开名（别名优先）。 */
  publicName: string;
  /** 已选分组中提供该模型的分组，按分组顺序。 */
  sourceGroups: string[];
  binding: AicliModelBinding | undefined;
  status: AicliPreviewStatus;
  protocols: string[];
  tags: string[];
}

function tagsOf(pricing: unknown): string[] {
  const tags =
    pricing && typeof pricing === "object"
      ? (pricing as { tags?: unknown }).tags
      : undefined;
  if (typeof tags !== "string") return [];
  return tags
    .split(",")
    .map((tag) => tag.trim())
    .filter(Boolean);
}

/** 合并后的模型表预览（含被排除的模型，标为 excluded）。 */
export function previewModels(
  body: AicliProfileBody,
  catalog: AicliCatalog | null | undefined,
): AicliPreviewRow[] {
  if (!catalog) return [];
  const rows: AicliPreviewRow[] = [];
  for (const [model, entry] of Object.entries(catalog.models)) {
    const sourceGroups = body.selected_groups.filter((group) =>
      entry.groups.includes(group),
    );
    if (sourceGroups.length === 0) continue;
    const binding = body.model_bindings.find((item) => item.model === model);
    let status: AicliPreviewStatus;
    if (body.excluded_models.includes(model)) status = "excluded";
    else if (binding) status = "bound";
    else if (sourceGroups.length === 1) status = "single";
    else if (body.resolved_conflicts.includes(model)) status = "confirmed";
    else status = "conflict";
    rows.push({
      model,
      publicName: binding?.alias || model,
      sourceGroups,
      binding,
      status,
      protocols: Object.keys(entry.wire),
      tags: tagsOf(entry.pricing),
    });
  }
  return rows.sort((a, b) => a.publicName.localeCompare(b.publicName));
}

export function unresolvedConflicts(
  body: AicliProfileBody,
  catalog: AicliCatalog | null | undefined,
): string[] {
  return previewModels(body, catalog)
    .filter((row) => row.status === "conflict")
    .map((row) => row.model);
}

/** 「按分组顺序全部确认」：把所有未解决冲突记为已确认（按分组顺序路由）。 */
export function confirmAllByGroupOrder(
  body: AicliProfileBody,
  catalog: AicliCatalog | null | undefined,
): AicliProfileBody {
  const pending = unresolvedConflicts(body, catalog);
  if (pending.length === 0) return body;
  return {
    ...body,
    resolved_conflicts: [...body.resolved_conflicts, ...pending],
  };
}

/** 冲突的处理方式：按分组顺序确认 / 指定来源分组 / 排除。 */
export type AicliConflictChoice =
  | { kind: "order" }
  | { kind: "source"; group: string }
  | { kind: "exclude" };

function without<T>(items: T[], predicate: (item: T) => boolean): T[] {
  return items.filter((item) => !predicate(item));
}

/** 应用一个模型的处理方式；别名保持不变（排除时别名一并去掉）。 */
export function applyChoice(
  body: AicliProfileBody,
  model: string,
  choice: AicliConflictChoice,
): AicliProfileBody {
  const alias =
    body.model_bindings.find((item) => item.model === model)?.alias ?? null;
  const bindings = without(body.model_bindings, (item) => item.model === model);
  const resolved = without(body.resolved_conflicts, (item) => item === model);
  const excluded = without(body.excluded_models, (item) => item === model);
  if (choice.kind === "exclude") {
    return {
      ...body,
      model_bindings: bindings,
      resolved_conflicts: resolved,
      excluded_models: [...excluded, model],
    };
  }
  if (choice.kind === "source") {
    return {
      ...body,
      model_bindings: [
        ...bindings,
        { model, source_group: choice.group, alias },
      ],
      resolved_conflicts: resolved,
      excluded_models: excluded,
    };
  }
  // 按分组顺序：有别名时仍保留一条只带别名的绑定，否则记为已确认
  return {
    ...body,
    model_bindings: alias
      ? [...bindings, { model, source_group: null, alias }]
      : bindings,
    resolved_conflicts: alias ? resolved : [...resolved, model],
    excluded_models: excluded,
  };
}

/** 设置或清除别名（空字符串 = 不起别名）。 */
export function applyAlias(
  body: AicliProfileBody,
  model: string,
  aliasInput: string,
): AicliProfileBody {
  const alias = aliasInput.trim() || null;
  const current = body.model_bindings.find((item) => item.model === model);
  const bindings = without(body.model_bindings, (item) => item.model === model);
  const source = current?.source_group ?? null;
  if (!alias && !source) {
    return { ...body, model_bindings: bindings };
  }
  return {
    ...body,
    model_bindings: [...bindings, { model, source_group: source, alias }],
    // 有绑定即已解决，不再需要确认记录
    resolved_conflicts: without(
      body.resolved_conflicts,
      (item) => item === model,
    ),
    excluded_models: without(body.excluded_models, (item) => item === model),
  };
}

/** 勾选或取消分组；取消时移除指向它的来源绑定（否则后端判为非法）。 */
export function toggleGroup(
  body: AicliProfileBody,
  group: string,
  selected: boolean,
): AicliProfileBody {
  if (selected) {
    if (body.selected_groups.includes(group)) return body;
    return { ...body, selected_groups: [...body.selected_groups, group] };
  }
  return {
    ...body,
    selected_groups: without(body.selected_groups, (item) => item === group),
    model_bindings: body.model_bindings.map((item) =>
      item.source_group === group ? { ...item, source_group: null } : item,
    ),
  };
}

/**
 * 保存前规范化：去掉不再出现在已选分组里的模型的绑定 / 确认 / 排除记录，
 * 以及既无来源也无别名的空绑定（后端对前者判 invalid_profile）。
 */
export function normalizeForSave(
  body: AicliProfileBody,
  catalog: AicliCatalog | null | undefined,
): AicliProfileBody {
  const available = new Set(
    previewModels(
      {
        ...body,
        excluded_models: [],
        model_bindings: [],
        resolved_conflicts: [],
      },
      catalog,
    ).map((row) => row.model),
  );
  return {
    ...body,
    model_bindings: body.model_bindings.filter(
      (item) => available.has(item.model) && (item.source_group || item.alias),
    ),
    resolved_conflicts: body.resolved_conflicts.filter((item) =>
      available.has(item),
    ),
    excluded_models: body.excluded_models.filter((item) => available.has(item)),
  };
}

export function moveGroup(
  body: AicliProfileBody,
  from: number,
  to: number,
): AicliProfileBody {
  const groups = [...body.selected_groups];
  if (to < 0 || to >= groups.length || from === to) return body;
  const [moved] = groups.splice(from, 1);
  groups.splice(to, 0, moved);
  return { ...body, selected_groups: groups };
}

/** 新建配置集的 id（本机唯一即可；后端只要求非空）。 */
export function newProfileId(existing: string[], now = Date.now()): string {
  let id = `user-${now.toString(36)}`;
  let n = 1;
  while (existing.includes(id)) id = `user-${now.toString(36)}-${n++}`;
  return id;
}

/** 从系统预设新建：复制预设内容为 system_copy，revision 0。 */
export function copyPreset(
  preset: AicliProfile,
  id: string,
  name: string,
): AicliProfile {
  return {
    id,
    name,
    source: "system_copy",
    preset_id: preset.preset_id ?? preset.id,
    revision: 0,
    body: {
      ...preset.body,
      selected_groups: [...preset.body.selected_groups],
      model_bindings: preset.body.model_bindings.map((item) => ({ ...item })),
      resolved_conflicts: [...preset.body.resolved_conflicts],
      excluded_models: [...(preset.body.excluded_models ?? [])],
    },
  };
}

/** 「查看差异」：预设的模型表与对照配置集相比，多出与缺少的公开模型名。 */
export function diffModelLists(
  preset: string[],
  other: string[],
): { onlyInPreset: string[]; onlyInOther: string[] } {
  const a = new Set(preset);
  const b = new Set(other);
  return {
    onlyInPreset: preset.filter((name) => !b.has(name)).sort(),
    onlyInOther: other.filter((name) => !a.has(name)).sort(),
  };
}

function knownRatio(
  catalog: AicliCatalog | null | undefined,
  group: string,
): number | null {
  const info = catalog?.groups[group];
  return info && info.ratio_known && info.ratio != null ? info.ratio : null;
}

/**
 * 实际生效的分组顺序。与后端 `profiles.rs::ordered_groups` 一致：cost_first 按倍率
 * 从低到高排（倍率未知的排最后，其余保持原顺序），否则就是用户排的顺序。
 */
export function effectiveGroupOrder(
  body: AicliProfileBody,
  catalog: AicliCatalog | null | undefined,
): string[] {
  if (body.selection_mode !== "cost_first") return body.selected_groups;
  return body.selected_groups
    .map((group, index) => ({
      group,
      index,
      ratio: knownRatio(catalog, group),
    }))
    .sort((a, b) => {
      if (a.ratio != null && b.ratio != null && a.ratio !== b.ratio)
        return a.ratio - b.ratio;
      if ((a.ratio == null) !== (b.ratio == null))
        return a.ratio == null ? 1 : -1;
      return a.index - b.index;
    })
    .map((item) => item.group);
}

export interface AicliLandingGroup {
  group: string;
  /** 首选来源落在这个分组的模型数（不含已排除的）。 */
  models: number;
  ratio: number | null;
  /**
   * 排在后面、却更便宜的分组：这里有 `models` 个模型在 `group` 里也有，倍率更低。
   * 只在按用户顺序选来源时给出；指定了来源的模型不算。
   */
  cheaper: { group: string; ratio: number; models: number } | null;
}

/** 「按这个顺序，模型会落在」：每个分组接住多少模型，以及有没有排在后面的更便宜来源。 */
export function landingGroups(
  body: AicliProfileBody,
  catalog: AicliCatalog | null | undefined,
): AicliLandingGroup[] {
  if (!catalog) return [];
  const order = effectiveGroupOrder(body, catalog);
  const counts = new Map<string, number>();
  // 落点分组 → 更便宜的后备分组 → 涉及的模型数
  const savings = new Map<string, Map<string, number>>();
  for (const row of previewModels(body, catalog)) {
    if (row.status === "excluded") continue;
    const sources = order.filter((group) => row.sourceGroups.includes(group));
    const pinned = row.binding?.source_group;
    const landing = pinned && sources.includes(pinned) ? pinned : sources[0];
    if (!landing) continue;
    counts.set(landing, (counts.get(landing) ?? 0) + 1);
    const ratio = knownRatio(catalog, landing);
    if (pinned || ratio == null || body.selection_mode === "cost_first")
      continue;
    for (const other of sources.slice(sources.indexOf(landing) + 1)) {
      const otherRatio = knownRatio(catalog, other);
      if (otherRatio == null || otherRatio >= ratio) continue;
      const perGroup = savings.get(landing) ?? new Map<string, number>();
      perGroup.set(other, (perGroup.get(other) ?? 0) + 1);
      savings.set(landing, perGroup);
    }
  }
  return order
    .filter((group) => counts.has(group))
    .map((group) => {
      let cheaper: AicliLandingGroup["cheaper"] = null;
      for (const [other, models] of savings.get(group) ?? []) {
        const ratio = knownRatio(catalog, other)!;
        if (!cheaper || ratio < cheaper.ratio)
          cheaper = { group: other, ratio, models };
      }
      return {
        group,
        models: counts.get(group)!,
        ratio: knownRatio(catalog, group),
        cheaper,
      };
    });
}

/** 把 `group` 移到 `before` 前面（「把更便宜的分组移到前面」）。 */
export function moveGroupBefore(
  body: AicliProfileBody,
  group: string,
  before: string,
): AicliProfileBody {
  const from = body.selected_groups.indexOf(group);
  const to = body.selected_groups.indexOf(before);
  if (from < 0 || to < 0 || from <= to) return body;
  return moveGroup(body, from, to);
}
