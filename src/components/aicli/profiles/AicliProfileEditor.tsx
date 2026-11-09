import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  closestCenter,
  DndContext,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import {
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { ArrowDown, ArrowUp, GripVertical, Loader2 } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Notice } from "@/components/ui/notice";
import { aicliErrorCode, isAicliCommandError } from "@/lib/api/aicli";
import { useAicliSaveProfile } from "@/hooks/useAicliProfiles";
import type {
  AicliCatalog,
  AicliProfile,
  AicliProfileBody,
} from "@/types/aicli";
import {
  applyAlias,
  applyChoice,
  confirmAllByGroupOrder,
  moveGroup,
  normalizeForSave,
  previewModels,
  toggleGroup,
  unresolvedConflicts,
  type AicliPreviewRow,
} from "./profileModel";

/** 后端校验错误码 → 文案键（aicli.profiles.errors.*）。 */
export const PROFILE_ERROR_KEYS: Record<string, string> = {
  unresolved_conflicts: "unresolvedConflicts",
  duplicate_alias: "duplicateAlias",
  missing_groups: "missingGroups",
  too_many_groups: "tooManyGroups",
  revision_conflict: "revisionConflict",
  invalid_profile: "invalidProfile",
  read_only: "readOnly",
  needs_repair: "needsRepair",
};

interface AicliProfileEditorProps {
  profile: AicliProfile;
  catalog: AicliCatalog | null | undefined;
  onSaved: (saved: AicliProfile) => void;
  onCancel: () => void;
}

function SortableGroup({
  id,
  index,
  count,
  ratio,
  onMove,
  onRemove,
}: {
  id: string;
  index: number;
  count: number;
  ratio: string;
  onMove: (from: number, to: number) => void;
  onRemove: () => void;
}) {
  const { t } = useTranslation();
  const { attributes, listeners, setNodeRef, transform, transition } =
    useSortable({ id });
  return (
    <li
      ref={setNodeRef}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      data-testid={`aicli-group-${id}`}
      className="flex items-center gap-2 rounded-control border border-border bg-surface px-2 py-1 text-body"
    >
      <button
        type="button"
        aria-label={t("aicli.profiles.dragGroup", { group: id })}
        className="cursor-grab text-fg-3"
        {...attributes}
        {...listeners}
      >
        <GripVertical aria-hidden="true" className="h-4 w-4" />
      </button>
      <span className="w-6 text-caption tabular-nums text-fg-3">
        {index + 1}
      </span>
      <span className="min-w-0 flex-1 truncate text-fg-1">{id}</span>
      <span className="text-caption text-fg-3">{ratio}</span>
      <Button
        type="button"
        variant="quiet"
        size="icon-compact"
        aria-label={t("aicli.profiles.moveUp", { group: id })}
        disabled={index === 0}
        onClick={() => onMove(index, index - 1)}
      >
        <ArrowUp aria-hidden="true" className="h-3.5 w-3.5" />
      </Button>
      <Button
        type="button"
        variant="quiet"
        size="icon-compact"
        aria-label={t("aicli.profiles.moveDown", { group: id })}
        disabled={index === count - 1}
        onClick={() => onMove(index, index + 1)}
      >
        <ArrowDown aria-hidden="true" className="h-3.5 w-3.5" />
      </Button>
      <Button type="button" variant="quiet" size="compact" onClick={onRemove}>
        {t("aicli.profiles.removeGroup")}
      </Button>
    </li>
  );
}

/**
 * 配置集编辑器（F-05）：勾选与拖动分组 → 预览合并后的模型表 → 多来源模型列为
 * 冲突（来源分组、倍率提示、协议、能力），逐项选来源 / 排除 / 起别名，或「按分组
 * 顺序全部确认」。有未解决冲突时保存不可用。
 */
export function AicliProfileEditor({
  profile,
  catalog,
  onSaved,
  onCancel,
}: AicliProfileEditorProps) {
  const { t } = useTranslation();
  const save = useAicliSaveProfile();
  const [name, setName] = useState(profile.name);
  const [body, setBody] = useState<AicliProfileBody>(profile.body);
  const [error, setError] = useState<string | null>(null);

  const sensors = useSensors(
    useSensor(PointerSensor),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    }),
  );

  const rows = useMemo(() => previewModels(body, catalog), [body, catalog]);
  const pending = useMemo(
    () => unresolvedConflicts(body, catalog),
    [body, catalog],
  );
  const allGroups = Object.keys(catalog?.groups ?? {});
  const unselected = allGroups.filter(
    (group) => !body.selected_groups.includes(group),
  );

  const ratioText = (group: string) => {
    const info = catalog?.groups[group];
    return info && info.ratio_known && info.ratio != null
      ? t("aicli.profiles.ratio", { ratio: info.ratio })
      : t("aicli.profiles.ratioUnknown");
  };

  const onDragEnd = (event: DragEndEvent) => {
    const { active, over } = event;
    if (!over || active.id === over.id) return;
    const from = body.selected_groups.indexOf(String(active.id));
    const to = body.selected_groups.indexOf(String(over.id));
    setBody((current) => moveGroup(current, from, to));
  };

  const submit = () => {
    setError(null);
    const next: AicliProfile = {
      ...profile,
      name: name.trim(),
      body: normalizeForSave(body, catalog),
    };
    save.mutate(next, {
      onSuccess: (saved) => onSaved(saved),
      onError: (failure) => {
        const key = PROFILE_ERROR_KEYS[aicliErrorCode(failure) ?? ""];
        if (key) {
          setError(t(`aicli.profiles.errors.${key}`));
          return;
        }
        setError(
          t("aicli.profiles.errors.other", {
            message: isAicliCommandError(failure)
              ? failure.message
              : String(failure),
          }),
        );
      },
    });
  };

  const statusText = (row: AicliPreviewRow) =>
    t(`aicli.profiles.status.${row.status}`);

  const choiceValue = (row: AicliPreviewRow) => {
    if (row.status === "excluded") return "exclude";
    if (row.binding?.source_group) return `source:${row.binding.source_group}`;
    if (row.status === "confirmed" || row.binding) return "order";
    return "";
  };

  const canSave = pending.length === 0 && name.trim() !== "" && !save.isPending;

  return (
    <div className="flex flex-col gap-4" data-testid="aicli-profile-editor">
      <label className="flex flex-col gap-1 text-body">
        <span className="text-fg-2">{t("aicli.profiles.name")}</span>
        <Input
          aria-label={t("aicli.profiles.name")}
          value={name}
          onChange={(event) => setName(event.target.value)}
        />
      </label>

      <section
        aria-labelledby="aicli-profile-groups"
        className="flex flex-col gap-2"
      >
        <h3 id="aicli-profile-groups" className="m-0 text-section text-fg-1">
          {t("aicli.profiles.groupsTitle")}
        </h3>
        <p className="m-0 text-caption text-fg-3">
          {t("aicli.profiles.groupsHint")}
        </p>
        <DndContext
          sensors={sensors}
          collisionDetection={closestCenter}
          onDragEnd={onDragEnd}
        >
          <SortableContext
            items={body.selected_groups}
            strategy={verticalListSortingStrategy}
          >
            <ol className="m-0 flex list-none flex-col gap-1 p-0">
              {body.selected_groups.map((group, index) => (
                <SortableGroup
                  key={group}
                  id={group}
                  index={index}
                  count={body.selected_groups.length}
                  ratio={ratioText(group)}
                  onMove={(from, to) =>
                    setBody((current) => moveGroup(current, from, to))
                  }
                  onRemove={() =>
                    setBody((current) => toggleGroup(current, group, false))
                  }
                />
              ))}
            </ol>
          </SortableContext>
        </DndContext>
        {unselected.length > 0 && (
          <div className="flex flex-wrap gap-2">
            {unselected.map((group) => (
              <label
                key={group}
                className="flex items-center gap-1.5 text-body text-fg-2"
              >
                <input
                  type="checkbox"
                  checked={false}
                  onChange={() =>
                    setBody((current) => toggleGroup(current, group, true))
                  }
                />
                {group}
                <span className="text-caption text-fg-3">
                  {ratioText(group)}
                </span>
              </label>
            ))}
          </div>
        )}
        <label className="flex items-center gap-2 text-body text-fg-2">
          <input
            type="checkbox"
            checked={body.allow_fallback}
            onChange={(event) =>
              setBody((current) => ({
                ...current,
                allow_fallback: event.target.checked,
              }))
            }
          />
          {t("aicli.profiles.allowFallback")}
        </label>
      </section>

      <section
        aria-labelledby="aicli-profile-preview"
        className="flex flex-col gap-2"
      >
        <div className="flex flex-wrap items-center gap-2">
          <h3 id="aicli-profile-preview" className="m-0 text-section text-fg-1">
            {t("aicli.profiles.previewTitle")}
          </h3>
          <Button
            type="button"
            variant="neutral"
            size="compact"
            className="ms-auto"
            disabled={pending.length === 0}
            onClick={() =>
              setBody((current) => confirmAllByGroupOrder(current, catalog))
            }
          >
            {t("aicli.profiles.confirmAll")}
          </Button>
        </div>
        {pending.length > 0 && (
          <Notice
            tone="warning"
            title={t("aicli.profiles.pendingConflicts", {
              count: pending.length,
            })}
          />
        )}
        <table className="w-full border-collapse text-body">
          <thead>
            <tr className="text-caption text-fg-3">
              <th className="py-1 pe-3 text-start font-normal">
                {t("aicli.profiles.columns.model")}
              </th>
              <th className="py-1 pe-3 text-start font-normal">
                {t("aicli.profiles.columns.sources")}
              </th>
              <th className="py-1 pe-3 text-start font-normal">
                {t("aicli.profiles.columns.protocols")}
              </th>
              <th className="py-1 pe-3 text-start font-normal">
                {t("aicli.profiles.columns.capabilities")}
              </th>
              <th className="py-1 pe-3 text-start font-normal">
                {t("aicli.profiles.columns.choice")}
              </th>
              <th className="py-1 text-start font-normal">
                {t("aicli.profiles.columns.alias")}
              </th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr
                key={row.model}
                data-testid={`aicli-preview-${row.model}`}
                data-status={row.status}
                className="border-t border-border align-top"
              >
                <td className="py-1.5 pe-3">
                  <div className="text-fg-1">{row.model}</div>
                  <div className="text-caption text-fg-3">
                    {statusText(row)}
                  </div>
                </td>
                <td className="py-1.5 pe-3 text-fg-2">
                  {row.sourceGroups
                    .map((group) => `${group}（${ratioText(group)}）`)
                    .join("、")}
                </td>
                <td className="py-1.5 pe-3 text-fg-2">
                  {row.protocols.join(", ")}
                </td>
                <td className="py-1.5 pe-3">
                  <span className="flex flex-wrap gap-1">
                    {row.tags.map((tag) => (
                      <Badge
                        key={tag}
                        variant="outline"
                        className="h-5 px-1.5 text-badge"
                      >
                        {tag}
                      </Badge>
                    ))}
                  </span>
                </td>
                <td className="py-1.5 pe-3">
                  {row.sourceGroups.length > 1 || row.status === "excluded" ? (
                    <select
                      aria-label={t("aicli.profiles.choiceFor", {
                        model: row.model,
                      })}
                      className="rounded-control border border-border-strong bg-surface px-1.5 py-0.5 text-body"
                      value={choiceValue(row)}
                      onChange={(event) => {
                        const value = event.target.value;
                        setBody((current) =>
                          applyChoice(
                            current,
                            row.model,
                            value === "exclude"
                              ? { kind: "exclude" }
                              : value.startsWith("source:")
                                ? { kind: "source", group: value.slice(7) }
                                : { kind: "order" },
                          ),
                        );
                      }}
                    >
                      <option value="" disabled>
                        {t("aicli.profiles.choicePlaceholder")}
                      </option>
                      <option value="order">
                        {t("aicli.profiles.choiceOrder")}
                      </option>
                      {row.sourceGroups.map((group) => (
                        <option key={group} value={`source:${group}`}>
                          {t("aicli.profiles.choiceSource", { group })}
                        </option>
                      ))}
                      <option value="exclude">
                        {t("aicli.profiles.choiceExclude")}
                      </option>
                    </select>
                  ) : (
                    <Button
                      type="button"
                      variant="quiet"
                      size="compact"
                      onClick={() =>
                        setBody((current) =>
                          applyChoice(current, row.model, { kind: "exclude" }),
                        )
                      }
                    >
                      {t("aicli.profiles.choiceExclude")}
                    </Button>
                  )}
                </td>
                <td className="py-1.5">
                  {row.status !== "excluded" && (
                    <Input
                      aria-label={t("aicli.profiles.aliasFor", {
                        model: row.model,
                      })}
                      placeholder={row.model}
                      value={row.binding?.alias ?? ""}
                      onChange={(event) =>
                        setBody((current) =>
                          applyAlias(current, row.model, event.target.value),
                        )
                      }
                      className="h-7"
                    />
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>

      {error && (
        <p role="alert" className="m-0 text-caption text-danger-text">
          {error}
        </p>
      )}
      <div className="flex items-center justify-end gap-2">
        <Button
          type="button"
          variant="neutral"
          size="regular"
          onClick={onCancel}
        >
          {t("aicli.profiles.cancel")}
        </Button>
        <Button
          type="button"
          variant="solid"
          size="regular"
          disabled={!canSave}
          onClick={submit}
        >
          {save.isPending && (
            <Loader2 aria-hidden="true" className="h-3.5 w-3.5 animate-spin" />
          )}
          {t("aicli.profiles.save")}
        </Button>
      </div>
    </div>
  );
}
