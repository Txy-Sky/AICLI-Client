import { fireEvent, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { UsageSettingsSheet } from "@/components/usage/UsageSettingsSheet";
import { renderWithQueryClient } from "../utils/testQueryClient";

const baseProps = {
  open: true,
  onOpenChange: vi.fn(),
  sessionAutoSyncEnabled: true,
  syncing: false,
  onSyncNow: vi.fn(),
  rebuildingCodex: false,
  onRebuildCodex: vi.fn(),
  refreshIntervalMs: 60_000,
  refreshIntervalOptions: [60_000] as const,
  onRefreshIntervalChange: vi.fn(),
  onOpenPricing: vi.fn(),
};

describe("UsageSettingsSheet — CLI session logs switch", () => {
  it("is off by default and reports the change", () => {
    const onChange = vi.fn();
    renderWithQueryClient(
      <UsageSettingsSheet
        {...baseProps}
        includeSessionLogs={false}
        onIncludeSessionLogsChange={onChange}
      />,
    );
    const toggle = screen.getByRole("switch", {
      name: "aicli.usage.includeSessionLogs",
    });
    expect(toggle).not.toBeChecked();
    expect(
      screen.getByText("aicli.usage.includeSessionLogsOff"),
    ).toBeInTheDocument();
    fireEvent.click(toggle);
    expect(onChange).toHaveBeenCalledWith(true);
  });

  it("says session logs are included when on", () => {
    renderWithQueryClient(
      <UsageSettingsSheet
        {...baseProps}
        includeSessionLogs
        onIncludeSessionLogsChange={vi.fn()}
      />,
    );
    expect(
      screen.getByRole("switch", { name: "aicli.usage.includeSessionLogs" }),
    ).toBeChecked();
    expect(
      screen.getByText("aicli.usage.includeSessionLogsOn"),
    ).toBeInTheDocument();
  });

  it("hides the switch when the page does not offer it", () => {
    renderWithQueryClient(<UsageSettingsSheet {...baseProps} />);
    expect(
      screen.queryByRole("switch", { name: "aicli.usage.includeSessionLogs" }),
    ).not.toBeInTheDocument();
  });
});
