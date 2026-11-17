import { describe, expect, it, vi } from "vitest";
import { checkForUpdate, UPDATER_ENABLED } from "@/lib/updater";

const mocks = vi.hoisted(() => ({ check: vi.fn(), getVersion: vi.fn() }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: mocks.check }));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: mocks.getVersion }));

describe("enabled AICLI updater", () => {
  it("asks the updater plugin and reports no update when there is none", async () => {
    mocks.getVersion.mockResolvedValue("0.1.0");
    mocks.check.mockResolvedValue(null);
    expect(UPDATER_ENABLED).toBe(true);
    expect(await checkForUpdate()).toEqual({ status: "up-to-date" });
    expect(mocks.check).toHaveBeenCalledTimes(1);
  });

  it("reports the version the updater plugin found", async () => {
    mocks.getVersion.mockResolvedValue("0.1.0");
    mocks.check.mockResolvedValue({
      version: "0.2.0",
      notes: "Release v0.2.0",
      date: "2026-11-15T00:00:00Z",
    });
    expect(await checkForUpdate()).toEqual({
      status: "available",
      info: {
        currentVersion: "0.1.0",
        availableVersion: "0.2.0",
        notes: "Release v0.2.0",
        pubDate: "2026-11-15T00:00:00Z",
      },
    });
  });
});
