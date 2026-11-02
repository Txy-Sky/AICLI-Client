import { describe, expect, it, vi } from "vitest";
import { checkForUpdate, UPDATER_ENABLED } from "@/lib/updater";

const mocks = vi.hoisted(() => ({ check: vi.fn(), getVersion: vi.fn() }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: mocks.check }));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: mocks.getVersion }));

describe("disabled AICLI updater", () => {
  it("does not invoke the updater or inspect a remote version", async () => {
    expect(UPDATER_ENABLED).toBe(false);
    expect(await checkForUpdate()).toEqual({ status: "up-to-date" });
    expect(mocks.check).not.toHaveBeenCalled();
    expect(mocks.getVersion).not.toHaveBeenCalled();
  });
});
