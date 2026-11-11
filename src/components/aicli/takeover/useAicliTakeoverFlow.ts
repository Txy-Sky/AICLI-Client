import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { AppId } from "@/lib/api";
import { aicliErrorCode, isAicliCommandError } from "@/lib/api/aicli";
import {
  useAicliRelease,
  useAicliTakeOver,
  useAicliTakeoverStatus,
} from "@/hooks/useAicliProfiles";

/** 支持 AICLI 接管的客户端（后端 mode::controller::PROXY_APPS）。 */
export const AICLI_TAKEOVER_APPS: readonly AppId[] = [
  "claude",
  "codex",
  "gemini",
  "grokbuild",
  "claude-desktop",
  // J 阶段的 Tier 2 客户端：与后端 `aicli/tier.rs` 的 TIER2_SUPPORTED 保持一致
  "opencode",
];

export type AicliTakeoverDisplay = "active" | "pending" | "inactive";

export function takeoverDisplay(
  state: string | undefined,
): AicliTakeoverDisplay {
  if (state === "active") return "active";
  if (state === "taking_over" || state === "releasing") return "pending";
  return "inactive";
}

export type AicliTakeoverDialogKind = "takeover" | "conflict" | "release";

export interface AicliTakeoverFlow {
  app: AppId;
  display: AicliTakeoverDisplay;
  /** 上一次接管遇到旧代理配置冲突（只在本次会话内知道，后端没有该状态）。 */
  conflict: boolean;
  busy: boolean;
  error: string | null;
  dialog: AicliTakeoverDialogKind | null;
  /** 打开确认对话框（接管 / 停止接管）。 */
  request: (kind: "takeover" | "release") => void;
  /** 确认对话框里的主按钮。 */
  confirm: () => void;
  cancel: () => void;
}

/**
 * 一个客户端的接管流程（F-04 客户端页与 F-08 客户端总览共用）：确认对话框说明会改写
 * 配置、停止接管还原；遇到旧代理配置冲突时二次确认覆盖。
 */
export function useAicliTakeoverFlow(
  app: AppId,
  enabled = true,
): AicliTakeoverFlow {
  const { t } = useTranslation();
  const status = useAicliTakeoverStatus(enabled);
  const takeOver = useAicliTakeOver();
  const release = useAicliRelease();
  const [dialog, setDialog] = useState<AicliTakeoverDialogKind | null>(null);
  const [conflict, setConflict] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const state = status.data?.find((entry) => entry.app === app)?.state;
  const busy = takeOver.isPending || release.isPending;

  const failMessage = (failure: unknown) =>
    t("aicli.client.failed", {
      message: isAicliCommandError(failure) ? failure.message : String(failure),
    });

  const runTakeOver = (confirmed: boolean) => {
    setError(null);
    takeOver.mutate(
      { app, confirmed },
      {
        onSuccess: () => {
          setConflict(false);
          setDialog(null);
        },
        onError: (failure) => {
          if (!confirmed && aicliErrorCode(failure) === "cc_switch_conflict") {
            setConflict(true);
            setDialog("conflict");
            return;
          }
          setDialog(null);
          setError(failMessage(failure));
        },
      },
    );
  };

  const runRelease = () => {
    setError(null);
    release.mutate(app, {
      onSettled: () => setDialog(null),
      onError: (failure) => setError(failMessage(failure)),
    });
  };

  return {
    app,
    display: takeoverDisplay(state),
    conflict,
    busy,
    error,
    dialog,
    request: (kind) => setDialog(kind),
    confirm: () =>
      dialog === "release" ? runRelease() : runTakeOver(dialog === "conflict"),
    cancel: () => {
      if (!busy) setDialog(null);
    },
  };
}
