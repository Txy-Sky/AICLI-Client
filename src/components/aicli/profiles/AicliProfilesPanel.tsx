import { useState } from "react";
import { useTranslation } from "react-i18next";
import { FullScreenPanel } from "@/components/common/FullScreenPanel";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Notice } from "@/components/ui/notice";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { APP_DISPLAY_NAME } from "@/components/shell/AppGlyph";
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

interface AicliProfilesPanelProps {
  isOpen: boolean;
  onClose: () => void;
}

type DiffTarget = { preset: AicliProfile; against: AicliRouteProfile | null };

/**
 * 配置集管理（F-05）：系统预设只读（从预设新建、查看差异），用户配置集可编辑、
 * 删除、设为默认或设为某客户端专用。配置集只存本机，不跨设备同步；不进入此页也能
 * 正常使用（默认推荐预设）。
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
        t("aicli.profiles.copyName", { name: preset.name }),
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

  return (
    <FullScreenPanel
      isOpen={isOpen}
      title={
        editing
          ? editing.name || t("aicli.profiles.title")
          : t("aicli.profiles.title")
      }
      onClose={() => {
        if (editing) setEditing(null);
        else onClose();
      }}
    >
      <div
        className="flex flex-col gap-4 px-6 pb-10 pt-4"
        data-testid="aicli-profiles-panel"
      >
        <Notice tone="neutral" title={t("aicli.profiles.localOnly")} />
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
              className="flex flex-col gap-2"
            >
              <h2 id="aicli-presets" className="m-0 text-section text-fg-1">
                {t("aicli.profiles.presetsTitle")}
              </h2>
              {(view?.presets ?? []).map((preset) => (
                <div
                  key={preset.id}
                  data-testid={`aicli-preset-${preset.id}`}
                  className="flex flex-col gap-2 rounded-panel border border-border bg-surface p-3"
                >
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="text-body text-fg-1">{preset.name}</span>
                    <Badge variant="outline" className="h-5 px-1.5 text-badge">
                      {t("aicli.profiles.readOnly")}
                    </Badge>
                    {globalId === preset.id && (
                      <Badge
                        variant="outline"
                        className="h-5 px-1.5 text-badge"
                      >
                        {t("aicli.profiles.isDefault")}
                      </Badge>
                    )}
                    <div className="ms-auto flex gap-2">
                      <Button
                        type="button"
                        variant="neutral"
                        size="compact"
                        onClick={() =>
                          setDiff({
                            preset,
                            against: routeOf(globalId) ?? null,
                          })
                        }
                      >
                        {t("aicli.profiles.viewDiff")}
                      </Button>
                      <Button
                        type="button"
                        variant="neutral"
                        size="compact"
                        onClick={() => startFromPreset(preset)}
                      >
                        {t("aicli.profiles.newFromPreset")}
                      </Button>
                    </div>
                  </div>
                  {repairNotice(preset.id)}
                </div>
              ))}
            </section>

            <section
              aria-labelledby="aicli-user-profiles"
              className="flex flex-col gap-2"
            >
              <h2
                id="aicli-user-profiles"
                className="m-0 text-section text-fg-1"
              >
                {t("aicli.profiles.userTitle")}
              </h2>
              {(view?.profiles ?? []).length === 0 && (
                <p className="m-0 text-body text-fg-2">
                  {t("aicli.profiles.noUserProfiles")}
                </p>
              )}
              {(view?.profiles ?? []).map((profile) => {
                const apps = appsBoundTo(profile.id);
                return (
                  <div
                    key={profile.id}
                    data-testid={`aicli-profile-${profile.id}`}
                    className="flex flex-col gap-2 rounded-panel border border-border bg-surface p-3"
                  >
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="text-body text-fg-1">
                        {profile.name}
                      </span>
                      {globalId === profile.id && (
                        <Badge
                          variant="outline"
                          className="h-5 px-1.5 text-badge"
                        >
                          {t("aicli.profiles.isDefault")}
                        </Badge>
                      )}
                      {apps.map((app) => (
                        <Badge
                          key={app}
                          variant="outline"
                          className="h-5 px-1.5 text-badge"
                        >
                          {APP_DISPLAY_NAME[
                            app as keyof typeof APP_DISPLAY_NAME
                          ] ?? app}
                        </Badge>
                      ))}
                      <div className="ms-auto flex flex-wrap gap-2">
                        <Button
                          type="button"
                          variant="neutral"
                          size="compact"
                          onClick={() => setEditing(profile)}
                        >
                          {t("aicli.profiles.edit")}
                        </Button>
                        <Button
                          type="button"
                          variant="neutral"
                          size="compact"
                          disabled={
                            globalId === profile.id || setActive.isPending
                          }
                          onClick={() =>
                            setActive.mutate(
                              { scope: "global", scopeKey: "", id: profile.id },
                              { onError: fail },
                            )
                          }
                        >
                          {t("aicli.profiles.setDefault")}
                        </Button>
                        <select
                          aria-label={t("aicli.profiles.useForClient", {
                            name: profile.name,
                          })}
                          className="rounded-control border border-border-strong bg-surface px-1.5 py-0.5 text-body"
                          value=""
                          onChange={(event) => {
                            const app = event.target.value;
                            if (!app) return;
                            setActive.mutate(
                              { scope: "app", scopeKey: app, id: profile.id },
                              { onError: fail },
                            );
                          }}
                        >
                          <option value="">
                            {t("aicli.profiles.useForClientPlaceholder")}
                          </option>
                          {AICLI_TAKEOVER_APPS.map((app) => (
                            <option key={app} value={app}>
                              {APP_DISPLAY_NAME[app]}
                            </option>
                          ))}
                        </select>
                        <Button
                          type="button"
                          variant="neutral"
                          size="compact"
                          onClick={() => setDeleting(profile)}
                        >
                          {t("aicli.profiles.delete")}
                        </Button>
                      </div>
                    </div>
                    {repairNotice(profile.id)}
                  </div>
                );
              })}
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
              {t("aicli.profiles.deleteTitle", { name: deleting?.name ?? "" })}
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
                preset: diff?.preset.name ?? "",
                other: diff?.against?.name ?? t("aicli.profile.recommended"),
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
