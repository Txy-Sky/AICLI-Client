import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { sessionsApi, settingsApi } from "@/lib/api";
import { toast } from "@/lib/toast";
import type { DeleteSessionOptions } from "@/lib/api/sessions";
import type { SessionMeta, Settings } from "@/types";
import { extractErrorMessage } from "@/utils/errorUtils";
import { sessionKeys } from "@/lib/query/sessions";

// G-03（2026-11-10）：供应商增删改的 mutation 已随 open_provider_terminal/
// add_provider 等对外命令删除；这里只保留与会话、设置相关的 mutation。

export const useDeleteSessionMutation = () => {
  const queryClient = useQueryClient();
  const { t } = useTranslation();

  return useMutation({
    mutationFn: async (input: DeleteSessionOptions) => {
      await sessionsApi.delete(input);
      return input;
    },
    onSuccess: async (input) => {
      const deleted = queryClient
        .getQueryData<SessionMeta[]>(["sessions"])
        ?.find(
          (session) =>
            session.providerId === input.providerId &&
            session.sessionId === input.sessionId &&
            session.sourcePath === input.sourcePath,
        );
      const deletedTitle = deleted?.title?.trim() || deleted?.summary?.trim();
      queryClient.setQueryData<SessionMeta[]>(["sessions"], (current) =>
        (current ?? []).filter(
          (session) =>
            !(
              session.providerId === input.providerId &&
              session.sessionId === input.sessionId &&
              session.sourcePath === input.sourcePath
            ),
        ),
      );
      queryClient.removeQueries({
        queryKey: sessionKeys.messages(input.providerId, input.sourcePath),
      });
      queryClient.removeQueries({
        queryKey: sessionKeys.transcript(input.providerId, input.sourcePath),
      });

      await queryClient.invalidateQueries({ queryKey: ["sessions"] });

      toast.success(
        deletedTitle
          ? t("sessionManager.sessionDeletedNamed", { title: deletedTitle })
          : t("sessionManager.sessionDeleted", {
              defaultValue: "会话已删除",
            }),
      );
    },
    onError: (error: Error) => {
      const detail = extractErrorMessage(error) || t("common.unknown");
      toast.error(
        t("sessionManager.deleteFailed", {
          defaultValue: "删除会话失败: {{error}}",
          error: detail,
        }),
      );
    },
  });
};

export const useSaveSettingsMutation = () => {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (settings: Settings) => {
      await settingsApi.save(settings);
    },
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings"] });
      await queryClient.invalidateQueries({
        queryKey: ["opencode", "runtime-models"],
      });
    },
  });
};
