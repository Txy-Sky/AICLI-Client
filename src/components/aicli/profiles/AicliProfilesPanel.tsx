import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowLeft,
  CopyPlus,
  Layers,
  Pencil,
  PiggyBank,
  Sparkles,
} from "lucide-react";
import { FullScreenPanel } from "@/components/common/FullScreenPanel";
import { AppPageHeader } from "@/components/shell/AppPageHeader";
import { Button } from "@/components/ui/button";
import { HoverTip } from "@/components/ui/hover-tip";
import { fieldClass } from "@/components/ui/input";
import { Notice } from "@/components/ui/notice";
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
import {
  AICLI_TAKEOVER_APPS,
  takeoverDisplay,
} from "@/components/aicli/AicliClientPage";
import { isAicliCommandError } from "@/lib/api/aicli";
import {
  useAicliCatalog,
  useAicliDeleteProfile,
  useAicliProfiles,
  useAicliRoutes,
  useAicliSetActiveProfile,
  useAicliTakeoverStatus,
} from "@/hooks/useAicliProfiles";
import type { AicliProfile, AicliRouteProfile } from "@/types/aicli";
import { AicliProfileEditor } from "./AicliProfileEditor";
import { copyPreset, diffModelLists, newProfileId } from "./profileModel";
import { aicliPresetDescription, aicliProfileName } from "./profileName";

interface AicliProfilesPanelProps {
  /** variant="page" 时忽略 */
  isOpen?: boolean;
  /** 二级页的返回；variant="page" 时只在编辑中用不到 */
  onClose?: () => void;
  /** panel：从首页 / 客户端页打开的二级页；page：侧栏「配置集」页 */
  variant?: "panel" | "page";
}

type DiffTarget = { preset: AicliProfile; against: AicliRouteProfile | null };

/**
 * 配置集管理（F-05；2026-11 改版，设计稿画板 4a）：上半「谁在用哪份」——全局默认
 * 和每个客户端各一个下拉；下半「全部配置集」——系统预设只读（复制一份来改、查看
 * 差异），自己的可编辑、删除，操作都直接摆在卡片上。配置集只存本机，不跨设备同步；
 * 不进入此页也能正常使用（默认推荐预设）。
 */
export function AicliProfilesPanel({
  isOpen: isOpenProp = false,
  onClose = () => undefined,
  variant = "panel",
}: AicliProfilesPanelProps) {
  const { t } = useTranslation();
  const isOpen = variant === "page" || isOpenProp;
  const profiles = useAicliProfiles(isOpen);
  const routes = useAicliRoutes(isOpen);
  const catalog = useAicliCatalog(isOpen);
  const takeover = useAicliTakeoverStatus(isOpen);
  const remove = useAicliDeleteProfile();
  const setActive = useAicliSetActiveProfile();
  const [editing, setEditing] = useState<AicliProfile | null>(null);
  const [deleting, setDeleting] = useState<AicliProfile | null>(null);
  const [diff, setDiff] = useState<DiffTarget | null>(null);
  const [error, setError] = useState<string | null>(null);

  const view = profiles.data;
  const all = [...(view?.presets ?? []), ...(view?.profiles ?? [])];
  const routeOf = (id: string) =>
    routes.data?.profiles.find((entry) => entry.id === id);
  const globalId =
    view?.bindings.find((binding) => binding.scope === "global")?.profile_id ??
    view?.active_profile_id ??
    "recommended";
  const globalName = aicliProfileName(
    t,
    all.find((profile) => profile.id === globalId),
    globalId,
  );
  /** 客户端单独指定的配置集；没有指定（跟随默认）时为 null。 */
  const appProfileId = (app: string) =>
    view?.bindings.find(
      (binding) => binding.scope === "app" && binding.scope_key === app,
    )?.profile_id ?? null;
  const isActive = (app: string) =>
    takeoverDisplay(
      (takeover.data ?? []).find((row) => row.app === app)?.state,
    ) === "active";
  /** 已接管、并且实际在用这份配置集的客户端。 */
  const activeAppsUsing = (id: string) =>
    AICLI_TAKEOVER_APPS.filter(
      (app) => isActive(app) && (appProfileId(app) ?? globalId) === id,
    );

  const fail = (failure: unknown) =>
    setError(
      t("aicli.profiles.errors.other", {
        message: isAicliCommandError(failure)
          ? failure.message
          : String(failure),
      }),
    );

  const assign = (
    scope: "global" | "app",
    scopeKey: string,
    id: string | null,
  ) => {
    setError(null);
    setActive.mutate({ scope, scopeKey, id }, { onError: fail });
  };

  const startFromPreset = (preset: AicliProfile) => {
    const ids = all.map((item) => item.id);
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
      <p className="m-0 text-caption tabular-nums text-fg-2">
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

  const selectClass = cn(
    fieldClass,
    "h-8 w-auto min-w-[200px] max-w-full pe-7",
  );

  const whoRow =
    "flex flex-wrap items-center justify-between gap-x-4 gap-y-2 px-4 py-2.5";

  const whoSection = (
    <section
      aria-labelledby="aicli-profiles-who"
      className="flex flex-col gap-2"
    >
      <h2 id="aicli-profiles-who" className="m-0 text-section text-fg-1">
        {t("aicli.profiles.whoTitle")}
      </h2>
      <div className="overflow-hidden rounded-panel border border-border bg-surface">
        <div
          data-testid="aicli-scope-global"
          className={cn(whoRow, "border-b border-border bg-action-soft")}
        >
          <span className="flex min-w-0 flex-col">
            <span className="text-strong font-semibold text-fg-1">
              {t("aicli.profiles.globalDefault")}
            </span>
            <span className="text-caption text-fg-2">
              {t("aicli.profiles.globalDefaultHint")}
            </span>
          </span>
          <select
            aria-label={t("aicli.profiles.globalDefault")}
            className={selectClass}
            value={globalId}
            disabled={setActive.isPending || all.length === 0}
            onChange={(event) => assign("global", "", event.target.value)}
          >
            {all.map((profile) => (
              <option key={profile.id} value={profile.id}>
                {aicliProfileName(t, profile)}
              </option>
            ))}
          </select>
        </div>
        {AICLI_TAKEOVER_APPS.map((app) => {
          const name = APP_DISPLAY_NAME[app] ?? app;
          const active = isActive(app);
          return (
            <div
              key={app}
              data-testid={`aicli-scope-${app}`}
              className={cn(whoRow, "border-b border-border last:border-b-0")}
            >
              <span className="flex min-w-0 items-center gap-2.5">
                <AppGlyph
                  app={app as AppId}
                  size={18}
                  badgeClassName="bg-surface"
                />
                <span className="flex min-w-0 flex-col">
                  <span className="truncate text-strong text-fg-1">{name}</span>
                  <span
                    className={cn(
                      "text-caption",
                      active ? "text-action-text" : "text-fg-3",
                    )}
                  >
                    {active
                      ? t("aicli.profiles.clientActive")
                      : t("aicli.profiles.clientInactive")}
                  </span>
                </span>
              </span>
              <select
                aria-label={t("aicli.profiles.profileFor", { app: name })}
                className={selectClass}
                value={appProfileId(app) ?? ""}
                disabled={setActive.isPending || all.length === 0}
                onChange={(event) =>
                  assign("app", app, event.target.value || null)
                }
              >
                <option value="">
                  {t("aicli.profiles.followDefault", { name: globalName })}
                </option>
                {all.map((profile) => (
                  <option key={profile.id} value={profile.id}>
                    {aicliProfileName(t, profile)}
                  </option>
                ))}
              </select>
            </div>
          );
        })}
      </div>
    </section>
  );

  const pill = "inline-flex h-5 items-center rounded-full px-2 text-badge";

  const card = (profile: AicliProfile, preset: boolean) => {
    const name = aicliProfileName(t, profile);
    const description = aicliPresetDescription(t, profile.id);
    const isDefault = globalId === profile.id;
    const users = activeAppsUsing(profile.id);
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
          "flex min-w-0 flex-col gap-2.5 rounded-panel bg-surface p-4",
          // 默认那份用 2px 品牌色描边；其余 1px，内边距补 1px 让内容对齐
          isDefault
            ? "border-2 border-action"
            : "border border-border p-[17px] transition-colors hover:border-border-strong",
        )}
      >
        <div className="flex min-w-0 flex-wrap items-center gap-1.5">
          {Icon && (
            <Icon
              aria-hidden="true"
              className="h-4 w-4 shrink-0 text-fg-2"
              strokeWidth={1.75}
            />
          )}
          <h3 className="m-0 min-w-0 truncate text-title text-fg-1">{name}</h3>
          {isDefault && (
            <span
              className={cn(
                pill,
                "bg-action-soft font-medium text-action-text",
              )}
            >
              {t("aicli.profiles.isDefault")}
            </span>
          )}
          <span
            className={cn(
              pill,
              preset ? "border border-border text-fg-2" : "bg-subtle text-fg-2",
            )}
          >
            {preset
              ? t("aicli.profiles.readOnly")
              : t("aicli.profiles.mineBadge")}
          </span>
        </div>
        {description && (
          <p className="m-0 text-caption text-fg-2">{description}</p>
        )}
        {stats(profile)}
        {groupChips(profile.body.selected_groups)}
        {users.length > 0 ? (
          <p className="m-0 flex flex-wrap items-center gap-x-2 gap-y-1 text-caption text-action-text">
            {t("aicli.profiles.inUse")}
            {users.map((app) => (
              <span key={app} className="inline-flex items-center gap-1">
                <AppGlyph
                  app={app as AppId}
                  size={14}
                  badgeClassName="bg-surface"
                />
                {APP_DISPLAY_NAME[app] ?? app}
              </span>
            ))}
          </p>
        ) : (
          <p className="m-0 text-caption text-fg-3">
            {t("aicli.profiles.notInUse")}
          </p>
        )}
        {repairNotice(profile.id)}
        <div className="mt-auto flex flex-wrap gap-2 pt-1">
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
          {!isDefault && (
            <Button
              type="button"
              variant="neutral"
              size="compact"
              disabled={setActive.isPending}
              onClick={() => assign("global", "", profile.id)}
            >
              {t("aicli.profiles.setDefault")}
            </Button>
          )}
          {preset ? (
            <Button
              type="button"
              variant="quiet"
              size="compact"
              onClick={() =>
                setDiff({
                  preset: profile,
                  against: routeOf(globalId) ?? null,
                })
              }
            >
              {t("aicli.profiles.viewDiff")}
            </Button>
          ) : (
            <Button
              type="button"
              variant="quiet"
              size="compact"
              className="text-danger-text hover:text-danger-text"
              onClick={() => setDeleting(profile)}
            >
              {t("aicli.profiles.delete")}
            </Button>
          )}
        </div>
      </article>
    );
  };

  const title = editing
    ? aicliProfileName(t, editing) || t("aicli.profiles.title")
    : t("aicli.profiles.title");
  const back = () => {
    if (editing) setEditing(null);
    else onClose();
  };
  const content = (
    <>
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
            <p className="m-0 text-body text-fg-2">
              <span>{t("aicli.profiles.intro")}</span>{" "}
              <span>{t("aicli.profiles.localOnly")}</span>
            </p>

            {whoSection}

            <section
              aria-labelledby="aicli-profiles-all"
              className="flex flex-col gap-2"
            >
              <h2
                id="aicli-profiles-all"
                className="m-0 text-section text-fg-1"
              >
                {t("aicli.profiles.allTitle")}
              </h2>
              <div className="grid grid-cols-[repeat(auto-fill,minmax(260px,1fr))] gap-3">
                {(view?.presets ?? []).map((preset) => card(preset, true))}
                {(view?.profiles ?? []).map((profile) => card(profile, false))}
              </div>
            </section>
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
    </>
  );

  // 侧栏「配置集」页（2026-11 改版）：页头 + 滚动区；编辑某份时页头左侧给返回
  if (variant === "page") {
    return (
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        <AppPageHeader
          icon={<Layers className="h-5 w-5" strokeWidth={1.5} />}
          leading={
            editing ? (
              <Button
                variant="quiet"
                size="icon-compact"
                className="h-8 w-8"
                aria-label={t("aicli.profiles.backToList")}
                onClick={back}
              >
                <ArrowLeft className="h-4 w-4" />
              </Button>
            ) : undefined
          }
          title={title}
        />
        <div
          id="main-content"
          className="min-h-0 flex-1 overflow-y-auto scroll-stable"
        >
          {content}
        </div>
      </div>
    );
  }

  return (
    <FullScreenPanel isOpen={isOpen} title={title} onClose={back}>
      {content}
    </FullScreenPanel>
  );
}
