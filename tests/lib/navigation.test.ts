import { afterEach, describe, expect, it } from "vitest";
import {
  appPageBelongsTo,
  isAppPage,
  isGlobalPage,
  parseView,
  readStoredView,
  VIEW_STORAGE_KEY,
} from "@/lib/navigation";

afterEach(() => {
  localStorage.removeItem(VIEW_STORAGE_KEY);
});

describe("navigation (F-02)", () => {
  it.each([
    null,
    "providers",
    "auth",
    "client",
    "skills",
    "skillsDiscovery",
    "settings",
    "garbage",
  ])("cold start is always home (stored %s)", (stored) => {
    if (stored !== null) localStorage.setItem(VIEW_STORAGE_KEY, stored);
    expect(readStoredView()).toBe("home");
  });

  it("maps legacy view names", () => {
    expect(parseView("providers")).toBe("home");
    expect(parseView("auth")).toBe("home");
    expect(parseView("openclawEnv")).toBe("openclawConfig");
    expect(parseView("client")).toBe("client");
    expect(parseView("usage")).toBe("usage");
    expect(parseView("nonsense")).toBeNull();
  });

  it("renames the app page to client and drops the auth page", () => {
    expect(isAppPage("client")).toBe(true);
    expect(isGlobalPage("usage")).toBe(true);
    // 2026-11 改版：首页、配置集是单独的全局页
    expect(isGlobalPage("home")).toBe(true);
    expect(isGlobalPage("profiles")).toBe(true);
    // 「auth」不再是全局页（类型上也删了）
    expect(isGlobalPage("auth" as never)).toBe(false);
  });

  it("keeps app-only pages with their own app", () => {
    expect(appPageBelongsTo("client", "codex")).toBe(true);
    expect(appPageBelongsTo("workspace", "openclaw")).toBe(true);
    expect(appPageBelongsTo("workspace", "codex")).toBe(false);
    expect(appPageBelongsTo("hermesMemory", "hermes")).toBe(true);
    expect(appPageBelongsTo("hermesMemory", "claude")).toBe(false);
  });
});
