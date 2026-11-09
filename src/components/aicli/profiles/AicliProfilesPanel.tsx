import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  CopyPlus,
  Layers,
  MoreHorizontal,
  Pencil,
  PiggyBank,
  Sparkles,
} from "lucide-react";
import { FullScreenPanel } from "@/components/common/FullScreenPanel";
import { Button } from "@/components/ui/button";
import { HoverTip } from "@/components/ui/hover-tip";
import { Notice } from "@/components/ui/notice";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { APP_DISPLAY_NAME, AppGlyph } from "@/components/shell/AppGlyph";
import type { AppId } from "@/lib/api";
import { cn } from "@/lib/utils";
import { AICLI_TAKEOVER_APPS } from "@/components/aicli/AicliClientPage";
import { isAicliCommandError } from "@/lib/api/aicli";
import {
  useAicliCatalog,
  useAicliDeleteProfile,
  useAicliProfiles,
  useAicliRoutes,
  useAicliSetActiveProfile,
} from "@/hooks/useAicliProfiles";
import type { AicliProfile, AicliRouteProfile } from "@/types/aicli";
import { AicliProfileEditor } from "./AicliProfileEditor";
import { copyPreset, diffModelLists, newProfileId } from "./profileModel";
import { aicliPresetDescription, aicliProfileName } from "./profileName";

interface AicliProfilesPanelProps {
  isOpen: boolean;
  onClose: () => void;
}

type DiffTarget = { preset: AicliProfile; against: AicliRouteProfile | null };

/**
 * 配置集管理（F-05，F-11 改为卡片）：系统预设只读（从预设新建、查看差异），用户
 * 配置集可编辑、删除、设为默认或设为某客户端专用。每张卡片一个主操作，其余在「更多」
 * 菜单里。配置集只存本机，不跨设备同步；不进入此页也能正常使用（默认推荐预设）。
 */
export function AicliProfilesPanel({
  isOpen,
  onClose,
}: AicliProfilesPanelProps) {
  const { t } = useTranslation();
  const profiles = useAicliProfiles(isOpen);
  const routes = useAicliRoutes(isOpen);
  const catalog = useAicliCatalog(isOpen);
  const remove = useAicliDeleteProfile();
  const setActive = useAicliSetActiveProfile();
  const [editing, setEditing] = useState<AicliProfile | null>(null);
  const [deleting, setDeleting] = useState<AicliProfile | null>(null);
  const [diff, setDiff] = useState<DiffTarget | null>(null);
  const [error, setError] = useState<string | null>(null);

  const view = profiles.data;
  const routeOf = (id: string) =>
    routes.data?.profiles.find((entry) => entry.id === id);
  const globalId =
    view?.bindings.find((binding) => binding.scope === "global")?.profile_id ??
    view?.active_profile_id ??
    "recommended";
  const appsBoundTo = (id: string) =>
    (view?.bindings ?? [])
      .filter((binding) => binding.scope === "app" && binding.profile_id === id)
      .map((binding) => binding.scope_key);

  const fail = (failure: unknown) =>
    setError(
      t("aicli.profiles.errors.other", {
        message: isAicliCommandError(failure)
          ? failure.message
          : String(failure),
      }),
    );

  const startFromPreset = (preset: AicliProfile) => {
    const ids = [
      ...(view?.profiles ?? []).map((item) => item.id),
      ...(view?.presets ?? []).map((item) => item.id),
    ];
    setEditing(
      copyPreset(
        preset,
        newProfileId(ids),
        t("aicli.profiles.copyName", { name: aicliProfileName(t, preset) }),
      ),
    );
  };

  const repairNotice = (id: string) => {
    const repair = routeOf(id)?.repair;
    if (!repair?.needs_repair) return null;
    return (
      <Notice tone="warning" title={t("aicli.profiles.needsRepairTitle")}>
        {t("aicli.profiles.needsRepairBody", {
          groups: repair.missing_groups.join("、"),
          models: repair.affected_models.join("、"),
        })}
      </Notice>
    );
  };

  const diffResult = diff
    ? diffModelLists(
        (routeOf(diff.preset.id)?.routes ?? []).map((r) => r.public_model),
        (diff.against?.routes ?? []).map((r) => r.public_model),
      )
    : null;

  const groupChips = (groups: string[]) => {
    const shown = groups.slice(0, 4);
    return (
      <div className="flex flex-wrap gap-1">
        {shown.map((group) => {
          const info = catalog.data?.groups[group];
          const tip = [
            info?.description,
            info?.ratio_known && info.ratio != null
              ? t("aicli.profiles.ratio", { ratio: info.ratio })
              : null,
          ]
            .filter(Boolean)
            .join(" · ");
          const chip = (
            <span className="inline-flex h-5 items-center rounded-full bg-subtle px-2 text-badge text-fg-2">
              {group}
            </span>
          );
          return tip ? (
            <HoverTip key={group} content={tip}>
              {chip}
            </HoverTip>
          ) : (
            <span key={group}>{chip}</span>
          );
        })}
        {groups.length > shown.length && (
          <span className="inline-flex h-5 items-center rounded-full bg-subtle px-2 text-badge text-fg-3">
            {t("aicli.profiles.groupsMore", {
              count: groups.length - shown.length,
            })}
          </span>
        )}
      </div>
    );
  };

  const stats = (profile: AicliProfile) => {
    const models = routeOf(profile.id)?.routes.length;
    return (
      <p className="m-0 text-caption tabular-nums text-fg-3">
        {models == null
          ? t("aicli.profiles.groupCount", {
              count: profile.body.selected_groups.length,
            })
          : t("aicli.profiles.stats", {
              models,
              groups: profile.body.selected_groups.length,
            })}
      </p>
    );
  };

  const badges = (profile: AicliProfile, preset: boolean) => {
    const apps = appsBoundTo(profile.id);
    return (
      <>
        {globalId === profile.id && (
          <span className="inline-flex h-5 items-center rounded-full bg-success-soft px-2 text-badge font-medium text-success-text">
            {t("aicli.profiles.isDefault")}
          </span>
        )}
        {preset && (
          <span className="inline-flex h-5 items-center rounded-full border border-border px-2 text-badge text-fg-3">
            {t("aicli.profiles.readOnly")}
          </span>
        )}
        {apps.map((app) => (
          <HoverTip
            key={app}
            content={t("aicli.profiles.inUseBy", {
              app: APP_DISPLAY_NAME[app as AppId] ?? app,
            })}
          >
            <span
              aria-label={t("aicli.profiles.inUseBy", {
                app: APP_DISPLAY_NAME[app as AppId] ?? app,
              })}
              className="inline-flex h-5 items-center gap-1 rounded-full border border-border px-1.5 text-badge text-fg-2"
            >
              <AppGlyph
                app={app as AppId}
                size={12}
                badgeClassName="bg-surface"
              />
              {APP_DISPLAY_NAME[app as AppId] ?? app}
            </span>
          </HoverTip>
        ))}
      </>
    );
  };

  const moreMenu = (profile: AicliProfile, preset: boolean) => {
    const name = aicliProfileName(t, profile);
    const apps = appsBoundTo(profile.id);
    return (
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            type="button"
            variant="quiet"
            size="icon-compact"
            aria-label={t("aicli.profiles.moreActions", { name })}
          >
            <MoreHorizontal aria-hidden="true" className="h-4 w-4" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="min-w-[200px]">
          {preset ? (
            <DropdownMenuItem
              onSelect={() =>
                setDiff({
                  preset: profile,
                  against: routeOf(globalId) ?? null,
                })
              }
            >
              {t("aicli.profiles.viewDiff")}
            </DropdownMenuItem>
          ) : (
            <>
              <DropdownMenuItem
                disabled={globalId === profile.id || setActive.isPending}
                onSelect={() =>
                  setActive.mutate(
                    { scope: "global", scopeKey: "", id: profile.id },
                    { onError: fail },
                  )
                }
              >
                {t("aicli.profiles.setDefault")}
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuLabel className="text-caption font-normal text-fg-3">
                {t("aicli.profiles.useForClientTitle")}
              </DropdownMenuLabel>
              {AICLI_TAKEOVER_APPS.map((app) => (
                <DropdownMenuCheckboxItem
                  key={app}
                  checked={apps.includes(app)}
                  disabled={apps.includes(app) || setActive.isPending}
                  onSelect={() =>
                    setActive.mutate(
                      { scope: "app", scopeKey: app, id: profile.id },
                      { onError: fail },
                    )
                  }
                >
                  {APP_DISPLAY_NAME[app]}
                </DropdownMenuCheckboxItem>
              ))}
              <DropdownMenuSeparator />
              <DropdownMenuItem
                className="text-danger-text focus:text-danger-text"
                onSelect={() => setDeleting(profile)}
              >
                {t("aicli.profiles.delete")}
              </DropdownMenuItem>
            </>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
    );
  };

  const card = (profile: AicliProfile, preset: boolean) => {
    const name = aicliProfileName(t, profile);
    const description = aicliPresetDescription(t, profile.id);
    const Icon =
      profile.id === "low_cost"
        ? PiggyBank
        : profile.id === "recommended"
          ? Sparkles
          : null;
    return (
      <article
        key={profile.id}
        data-testid={`aicli-${preset ? "preset" : "profile"}-${profile.id}`}
        aria-label={name}
        className={cn(
          "flex min-w-0 flex-col gap-3 rounded-panel border bg-surface p-4 transition-colors hover:border-border-strong",
          globalId === profile.id ? "border-border-strong" : "border-border",
        )}
      >
        <div className="flex items-start gap-3">
          <span
            aria-hidden="true"
            className={cn(
              "flex h-9 w-9 shrink-0 items-center justify-center rounded-[10px] text-strong font-semibold",
              preset
                ? "bg-[color-mix(in_srgb,var(--chart-usage)_14%,transparent)] text-[var(--chart-usage)]"
                : "bg-subtle text-fg-2",
            )}
          >
            {Icon ? (
              <Icon className="h-[18px] w-[18px]" strokeWidth={1.75} />
            ) : (
              Array.from(name)[0]?.toUpperCase()
            )}
          </span>
          <div className="flex min-w-0 flex-1 flex-col gap-0.5">
            <div className="flex min-w-0 flex-wrap items-center gap-1.5">
              <h3 className="m-0 min-w-0 truncate text-strong text-fg-1">
                {name}
              </h3>
              {badges(profile, preset)}
            </div>
            {description && (
              <p className="m-0 text-caption text-fg-2">{description}</p>
            )}
            {stats(profile)}
          </div>
          {moreMenu(profile, preset)}
        </div>
        {groupChips(profile.body.selected_groups)}
        {repairNotice(profile.id)}
        <div className="mt-auto flex">
          <Button
            type="button"
            variant="neutral"
            size="compact"
            onClick={() =>
              preset ? startFromPreset(profile) : setEditing(profile)
            }
          >
            {preset ? (
              <CopyPlus aria-hidden="true" className="h-3.5 w-3.5" />
            ) : (
              <Pencil aria-hidden="true" className="h-3.5 w-3.5" />
            )}
            {preset
              ? t("aicli.profiles.newFromPreset")
              : t("aicli.profiles.edit")}
          </Button>
        </div>
      </article>
    );
  };

  const grid = "grid grid-cols-[repeat(auto-fill,minmax(260px,1fr))] gap-3";

  return (
    <FullScreenPanel
      isOpen={isOpen}
      title={
        editing
          ? aicliProfileName(t, editing) || t("aicli.profiles.title")
          : t("aicli.profiles.title")
      }
      onClose={() => {
        if (editing) setEditing(null);
        else onClose();
      }}
    >
      <div
        className="flex flex-col gap-6 px-6 pb-10 pt-4"
        data-testid="aicli-profiles-panel"
      >
        {error && (
          <p role="alert" className="m-0 text-caption text-danger-text">
            {error}
          </p>
        )}

        {editing ? (
          <AicliProfileEditor
            key={editing.id}
            profile={editing}
            catalog={catalog.data}
            onCancel={() => setEditing(null)}
            onSaved={() => setEditing(null)}
          />
        ) : (
          <>
            <section
              aria-labelledby="aicli-presets"
              className="flex flex-col gap-3"
            >
              <h2
                id="aicli-presets"
                className="m-0 text-caption font-semibold text-fg-2"
              >
                {t("aicli.profiles.presetsTitle")}
              </h2>
              <div className={grid}>
                {(view?.presets ?? []).map((preset) => card(preset, true))}
              </div>
            </section>

            <section
              aria-labelledby="aicli-user-profiles"
              className="flex flex-col gap-3"
            >
              <h2
                id="aicli-user-profiles"
                className="m-0 text-caption font-semibold text-fg-2"
              >
                {t("aicli.profiles.userTitle")}
              </h2>
              {(view?.profiles ?? []).length === 0 ? (
                <div className="flex flex-col items-center gap-1 rounded-panel border border-dashed border-border px-4 py-8 text-center">
                  <Layers
                    aria-hidden="true"
                    className="h-5 w-5 text-fg-3"
                    strokeWidth={1.5}
                  />
                  <p className="m-0 text-body text-fg-2">
                    {t("aicli.profiles.noUserProfiles")}
                  </p>
                </div>
              ) : (
                <div className={grid}>
                  {(view?.profiles ?? []).map((profile) =>
                    card(profile, false),
                  )}
                </div>
              )}
            </section>

            <p className="m-0 text-caption text-fg-3">
              {t("aicli.profiles.localOnly")}
            </p>
          </>
        )}
      </div>

      <Dialog
        open={deleting !== null}
        onOpenChange={(open) => !open && setDeleting(null)}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {t("aicli.profiles.deleteTitle", {
                name: deleting ? aicliProfileName(t, deleting) : "",
              })}
            </DialogTitle>
            <DialogDescription>
              {t("aicli.profiles.deleteBody")}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              type="button"
              variant="neutral"
              size="regular"
              onClick={() => setDeleting(null)}
            >
              {t("aicli.profiles.cancel")}
            </Button>
            <Button
              type="button"
              variant="solid"
              size="regular"
              disabled={remove.isPending}
              onClick={() => {
                if (!deleting) return;
                remove.mutate(
                  { id: deleting.id, revision: deleting.revision },
                  { onSettled: () => setDeleting(null), onError: fail },
                );
              }}
            >
              {t("aicli.profiles.delete")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={diff !== null}
        onOpenChange={(open) => !open && setDiff(null)}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {t("aicli.profiles.diffTitle", {
                preset: diff ? aicliProfileName(t, diff.preset) : "",
                other: aicliProfileName(t, diff?.against, "recommended"),
              })}
            </DialogTitle>
            <DialogDescription>
              {t("aicli.profiles.diffBody")}
            </DialogDescription>
          </DialogHeader>
          {diffResult &&
          diffResult.onlyInPreset.length === 0 &&
          diffResult.onlyInOther.length === 0 ? (
            <p className="m-0 text-body text-fg-2">
              {t("aicli.profiles.diffNone")}
            </p>
          ) : (
            <div className="flex flex-col gap-2 text-body">
              <div>
                <div className="text-caption text-fg-3">
                  {t("aicli.profiles.diffOnlyPreset")}
                </div>
                <div data-testid="aicli-diff-only-preset" className="text-fg-1">
                  {diffResult?.onlyInPreset.join("、") || "—"}
                </div>
              </div>
              <div>
                <div className="text-caption text-fg-3">
                  {t("aicli.profiles.diffOnlyOther")}
                </div>
                <div data-testid="aicli-diff-only-other" className="text-fg-1">
                  {diffResult?.onlyInOther.join("、") || "—"}
                </div>
              </div>
            </div>
          )}
        </DialogContent>
      </Dialog>
    </FullScreenPanel>
  );
}
