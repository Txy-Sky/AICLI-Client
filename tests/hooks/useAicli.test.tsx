import type { ReactNode } from "react";
import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClientProvider, type QueryClient } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  aicliApi,
  aicliErrorCode,
  AICLI_EVENTS,
  isAicliCommandError,
} from "@/lib/api/aicli";
import { aicliKeys, keysForDataKind } from "@/hooks/aicliKeys";
import {
  useAicliAccount,
  useAicliBind,
  useAicliCachedValue,
  useAicliEventBridge,
} from "@/hooks/useAicliAccount";
import { useAicliSelectLine } from "@/hooks/useAicliLines";
import { useAicliRequestBill } from "@/hooks/useAicliUsage";
import { useAicliCatalog } from "@/hooks/useAicliProfiles";
import type { AicliAccountView, AicliProfile } from "@/types/aicli";
import { emitTauriEvent } from "../msw/tauriMocks";
import { createTestQueryClient } from "../utils/testQueryClient";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

const boundAccount: AicliAccountView = {
  state: "bound",
  user_id: 7,
  username: "fixture",
  user_group: "default",
  selected_line_id: "main",
  active_profile_id: null,
  managed_tokens: [],
};

function wrapperFor(client: QueryClient) {
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
}

function callsTo(command: string) {
  return invokeMock.mock.calls.filter(([name]) => name === command);
}

beforeEach(() => {
  invokeMock.mockReset();
});

describe("aicliApi", () => {
  const profile: AicliProfile = {
    id: "custom",
    name: "Custom",
    source: "user",
    preset_id: null,
    revision: 3,
    body: {
      selected_groups: ["default"],
      selection_mode: "fixed_priority",
      model_bindings: [],
      resolved_conflicts: [],
      line_policy: "selected",
      allow_fallback: false,
    },
  };

  // 每个 C3-11 命令：前端方法 → 命令名与参数（Tauri 约定 camelCase）
  const cases: Array<[string, () => Promise<unknown>, string, unknown]> = [
    ["getAccount", () => aicliApi.getAccount(), "aicli_get_account", undefined],
    [
      "bind",
      () => aicliApi.bind("access-value", "main"),
      "aicli_bind",
      { accessToken: "access-value", line: "main" },
    ],
    [
      "unbind",
      () => aicliApi.unbind(true),
      "aicli_unbind",
      { deleteCreatedTokens: true },
    ],
    [
      "refresh",
      () => aicliApi.refresh("all"),
      "aicli_refresh",
      { kind: "all" },
    ],
    ["getCatalog", () => aicliApi.getCatalog(), "aicli_get_catalog", undefined],
    ["getRoutes", () => aicliApi.getRoutes(), "aicli_get_routes", undefined],
    ["listLines", () => aicliApi.listLines(), "aicli_list_lines", undefined],
    [
      "selectLine",
      () => aicliApi.selectLine("backup"),
      "aicli_select_line",
      { id: "backup" },
    ],
    ["probeLines", () => aicliApi.probeLines(), "aicli_probe_lines", undefined],
    [
      "getProfiles",
      () => aicliApi.getProfiles(),
      "aicli_get_profiles",
      undefined,
    ],
    [
      "saveProfile",
      () => aicliApi.saveProfile(profile),
      "aicli_save_profile",
      { profile },
    ],
    [
      "deleteProfile",
      () => aicliApi.deleteProfile("custom", 3),
      "aicli_delete_profile",
      { id: "custom", revision: 3 },
    ],
    [
      "setActiveProfile",
      () => aicliApi.setActiveProfile("app", "codex", "custom"),
      "aicli_set_active_profile",
      { scope: "app", scopeKey: "codex", id: "custom" },
    ],
    [
      "takeoverStatus",
      () => aicliApi.takeoverStatus(),
      "aicli_takeover_status",
      undefined,
    ],
    [
      "takeOver",
      () => aicliApi.takeOver("claude", false),
      "aicli_take_over",
      { app: "claude", confirmed: false },
    ],
    [
      "release",
      () => aicliApi.release("claude"),
      "aicli_release",
      { app: "claude" },
    ],
    [
      "getUsageSummary",
      () => aicliApi.getUsageSummary(),
      "aicli_get_usage_summary",
      undefined,
    ],
    [
      "getRequestBill",
      () => aicliApi.getRequestBill("req-1"),
      "aicli_get_request_bill",
      { requestId: "req-1" },
    ],
    ["getPrivacy", () => aicliApi.getPrivacy(), "aicli_get_privacy", undefined],
    [
      "setPrivacy",
      () =>
        aicliApi.setPrivacy({
          privacyEnabled: true,
          privacyFailureMode: "block",
          allowUnscannedMedia: false,
        }),
      "aicli_set_privacy",
      {
        privacyEnabled: true,
        privacyFailureMode: "block",
        allowUnscannedMedia: false,
        privacyDetectCjkNames: null,
      },
    ],
  ];

  it.each(cases)(
    "%s invokes the matching command",
    async (_, call, command, args) => {
      invokeMock.mockResolvedValue(undefined);
      await call();
      expect(invokeMock).toHaveBeenCalledTimes(1);
      if (args === undefined) {
        expect(invokeMock).toHaveBeenCalledWith(command);
      } else {
        expect(invokeMock).toHaveBeenCalledWith(command, args);
      }
    },
  );

  it("covers every registered AICLI command", () => {
    const commands = new Set(cases.map(([, , command]) => command));
    expect(commands.size).toBe(20);
  });

  it("recognises AICLI command errors by their machine code", () => {
    const error = { code: "token_invalid", message: "Token rejected" };
    expect(isAicliCommandError(error)).toBe(true);
    expect(aicliErrorCode(error)).toBe("token_invalid");
    expect(isAicliCommandError(new Error("boom"))).toBe(false);
    expect(aicliErrorCode("boom")).toBeUndefined();
  });
});

describe("keysForDataKind", () => {
  it("maps cache kinds to the queries they feed", () => {
    expect(keysForDataKind("account")).toEqual([
      aicliKeys.account,
      aicliKeys.cache("account"),
    ]);
    expect(keysForDataKind("pricing")).toContainEqual(aicliKeys.catalog);
    expect(keysForDataKind("groups")).toContainEqual(aicliKeys.routes);
    expect(keysForDataKind("usage:2026-11-08")).toEqual([
      aicliKeys.usageSummary,
    ]);
    expect(keysForDataKind("something-new")).toEqual([aicliKeys.all]);
  });
});

describe("AICLI hooks", () => {
  it("returns the first cached value, or null when nothing is cached", async () => {
    const client = createTestQueryClient();
    invokeMock.mockResolvedValueOnce([
      {
        kind: "account",
        value: { data: { quota: 5 } },
        fetched_at: 1,
        stale: false,
        last_error: null,
      },
    ]);
    const { result } = renderHook(() => useAicliCachedValue("account"), {
      wrapper: wrapperFor(client),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data?.value).toEqual({ data: { quota: 5 } });
    expect(invokeMock).toHaveBeenCalledWith("aicli_refresh", {
      kind: "account",
    });

    const empty = createTestQueryClient();
    invokeMock.mockResolvedValueOnce([]);
    const second = renderHook(() => useAicliCachedValue("pricing"), {
      wrapper: wrapperFor(empty),
    });
    await waitFor(() => expect(second.result.current.isSuccess).toBe(true));
    expect(second.result.current.data).toBeNull();
  });

  it("does not query the cache while disabled (not bound)", async () => {
    const client = createTestQueryClient();
    renderHook(() => useAicliCachedValue("account", false), {
      wrapper: wrapperFor(client),
    });
    await act(async () => {
      await Promise.resolve();
    });
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("invalidates every AICLI query after binding", async () => {
    const client = createTestQueryClient();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "aicli_get_account") return boundAccount;
      if (command === "aicli_bind") {
        return {
          account: boundAccount,
          accountError: null,
          proxyStarted: true,
          proxyError: null,
          runtimeRefreshError: null,
          takeoverTargetsError: null,
          takeovers: [],
        };
      }
      return undefined;
    });
    const { result } = renderHook(
      () => ({ account: useAicliAccount(), bind: useAicliBind() }),
      { wrapper: wrapperFor(client) },
    );
    await waitFor(() => expect(result.current.account.isSuccess).toBe(true));
    expect(callsTo("aicli_get_account")).toHaveLength(1);

    await act(async () => {
      await result.current.bind.mutateAsync({
        accessToken: "access-value",
        line: "main",
      });
    });
    await waitFor(() => expect(callsTo("aicli_get_account")).toHaveLength(2));
    expect(callsTo("aicli_bind")[0][1]).toEqual({
      accessToken: "access-value",
      line: "main",
    });
  });

  it("refetches lines, account and routes after switching line", async () => {
    const client = createTestQueryClient();
    invokeMock.mockImplementation(async (command: string) =>
      command === "aicli_get_account" ? boundAccount : undefined,
    );
    const { result } = renderHook(
      () => ({ account: useAicliAccount(), select: useAicliSelectLine() }),
      { wrapper: wrapperFor(client) },
    );
    await waitFor(() => expect(result.current.account.isSuccess).toBe(true));
    await act(async () => {
      await result.current.select.mutateAsync("backup");
    });
    expect(invokeMock).toHaveBeenCalledWith("aicli_select_line", {
      id: "backup",
    });
    await waitFor(() => expect(callsTo("aicli_get_account")).toHaveLength(2));
  });

  it("refreshes only the queries fed by the updated cache kind", async () => {
    const client = createTestQueryClient();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "aicli_get_account") return boundAccount;
      if (command === "aicli_get_catalog") return null;
      return undefined;
    });
    const { result } = renderHook(
      () => {
        useAicliEventBridge();
        return { account: useAicliAccount(), catalog: useAicliCatalog() };
      },
      { wrapper: wrapperFor(client) },
    );
    await waitFor(() => {
      expect(result.current.account.isSuccess).toBe(true);
      expect(result.current.catalog.isSuccess).toBe(true);
    });
    // 监听注册是异步的：等一轮微任务再发事件
    await act(async () => {
      await Promise.resolve();
    });

    act(() => emitTauriEvent(AICLI_EVENTS.dataUpdated, "pricing"));
    await waitFor(() => expect(callsTo("aicli_get_catalog")).toHaveLength(2));
    expect(callsTo("aicli_get_account")).toHaveLength(1);

    act(() => emitTauriEvent(AICLI_EVENTS.dataUpdated, "account"));
    await waitFor(() => expect(callsTo("aicli_get_account")).toHaveLength(2));
    expect(callsTo("aicli_get_catalog")).toHaveLength(2);
  });

  it("only fetches a request bill once a request id is known", async () => {
    const client = createTestQueryClient();
    invokeMock.mockResolvedValue(null);
    const { rerender } = renderHook(
      ({ id }: { id: string | null }) => useAicliRequestBill(id),
      {
        wrapper: wrapperFor(client),
        initialProps: { id: null as string | null },
      },
    );
    await act(async () => {
      await Promise.resolve();
    });
    expect(invokeMock).not.toHaveBeenCalled();

    rerender({ id: "req-9" });
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("aicli_get_request_bill", {
        requestId: "req-9",
      }),
    );
  });
});
