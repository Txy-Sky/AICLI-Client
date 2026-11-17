import type { TFunction } from "i18next";

/**
 * 系统预设的 id（后端 `profiles.rs::preset()`）。后端把预设的 name 直接设为 id，
 * 显示名和说明按界面语言从 `aicli.profiles.presets.<id>` 取（F-11）。
 */
export const AICLI_PRESET_IDS = ["recommended", "low_cost"] as const;
export type AicliPresetId = (typeof AICLI_PRESET_IDS)[number];

export function isAicliPresetId(
  id: string | null | undefined,
): id is AicliPresetId {
  return (AICLI_PRESET_IDS as readonly string[]).includes(id ?? "");
}

interface NamedProfile {
  id: string;
  name: string;
}

/**
 * 配置集显示名：系统预设按语言翻译，用户配置集用自己的名称；
 * 没有配置集对象时（还没取到）按 id 推断，预设同样翻译。
 */
export function aicliProfileName(
  t: TFunction,
  profile: NamedProfile | null | undefined,
  fallbackId?: string | null,
): string {
  const id = profile?.id ?? fallbackId ?? "recommended";
  if (isAicliPresetId(id)) return t(`aicli.profiles.presets.${id}.name`);
  return profile?.name?.trim() || id;
}

/** 系统预设的一句话说明；用户配置集没有。 */
export function aicliPresetDescription(
  t: TFunction,
  id: string | null | undefined,
): string | null {
  return isAicliPresetId(id) ? t(`aicli.profiles.presets.${id}.desc`) : null;
}
