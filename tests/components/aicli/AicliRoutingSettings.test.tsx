import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AicliRoutingSettings } from "@/components/aicli/AicliRoutingSettings";
import type { AicliLinesView, AicliPrivacyView } from "@/types/aicli";
import { renderWithQueryClient } from "../../utils/testQueryClient";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

const privacy = (patch: Partial<AicliPrivacyView> = {}): AicliPrivacyView => ({
  bound: true,
  privacy_enabled: true,
  privacy_failure_mode: "block",
  allow_unscanned_media: false,
  privacy_detect_cjk_names: false,
  privacy_detect_cjk_names_available: false,
  engine: null,
  custom_words: ["Nightjar"],
  custom_rules: [{ kind: "TICKET", regex: "TCK-\\d{6}" }],
  secret_backend: "keychain",
  ...patch,
});

const lines: AicliLinesView = {
  selected_line_id: "main",
  lines: [
    {
      id: "main",
      label_key: "aicli.lines.main",
      transport: "cloudflare",
      priority: 100,
      first_byte_s: 100,
      idle_s: 110,
      stream_total_s: 600,
      non_stream_s: 110,
      health: null,
    },
  ],
};

function respond(patch: Record<string, unknown> = {}) {
  const responses: Record<string, unknown> = {
    aicli_get_privacy: privacy(),
    aicli_list_lines: lines,
    aicli_takeover_status: [],
    ...patch,
  };
  invokeMock.mockImplementation(async (command: string) => {
    const reply = responses[command];
    if (typeof reply === "function") return (reply as () => unknown)();
    return reply;
  });
}

beforeEach(() => {
  invokeMock.mockReset();
});

describe("AicliRoutingSettings", () => {
  it("describes the lines with their timeouts and marks the current one", async () => {
    respond();
    renderWithQueryClient(<AicliRoutingSettings onOpenApp={vi.fn()} />);
    expect(await screen.findByText("aicli.lines.main")).toBeInTheDocument();
    expect(screen.getByText("aicli.settings.lineDetail")).toBeInTheDocument();
    expect(screen.getByText("aicli.lines.current")).toBeInTheDocument();
  });

  it("toggles privacy, failure mode and unscanned media through set_privacy", async () => {
    respond({ aicli_set_privacy: undefined });
    renderWithQueryClient(<AicliRoutingSettings onOpenApp={vi.fn()} />);
    fireEvent.click(
      await screen.findByRole("switch", {
        name: "aicli.settings.privacyEnabled",
      }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("aicli_set_privacy", {
        privacyEnabled: false,
        privacyFailureMode: "block",
        allowUnscannedMedia: false,
        privacyDetectCjkNames: null,
      }),
    );
    fireEvent.change(
      screen.getByRole("combobox", { name: "aicli.settings.failureMode" }),
      { target: { value: "pass" } },
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("aicli_set_privacy", {
        privacyEnabled: true,
        privacyFailureMode: "pass",
        allowUnscannedMedia: false,
        privacyDetectCjkNames: null,
      }),
    );
    fireEvent.click(
      screen.getByRole("switch", { name: "aicli.settings.allowMedia" }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("aicli_set_privacy", {
        privacyEnabled: true,
        privacyFailureMode: "block",
        allowUnscannedMedia: true,
        privacyDetectCjkNames: null,
      }),
    );
  });

  it("saves custom words and rules together", async () => {
    respond({ aicli_set_privacy_custom: undefined });
    renderWithQueryClient(<AicliRoutingSettings onOpenApp={vi.fn()} />);
    const words = await screen.findByRole("textbox", {
      name: "aicli.settings.customWords",
    });
    expect(words).toHaveValue("Nightjar");
    fireEvent.change(words, { target: { value: "Nightjar\n  代号 \n\n" } });
    fireEvent.click(
      screen.getByRole("button", { name: "aicli.settings.addRule" }),
    );
    const row = screen.getByTestId("aicli-rule-1");
    fireEvent.change(
      within(row).getByRole("textbox", { name: "aicli.settings.ruleKind" }),
      { target: { value: "acme key" } },
    );
    fireEvent.change(
      within(row).getByRole("textbox", { name: "aicli.settings.ruleValue" }),
      { target: { value: "ACME-" } },
    );
    fireEvent.click(
      screen.getByRole("button", { name: "aicli.settings.saveCustom" }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("aicli_set_privacy_custom", {
        words: ["Nightjar", "代号"],
        rules: [
          { kind: "TICKET", regex: "TCK-\\d{6}" },
          { kind: "acme key", prefix: "ACME-" },
        ],
      }),
    );
    expect(
      await screen.findByText("aicli.settings.customSaved"),
    ).toBeInTheDocument();
  });

  it("shows why custom items were rejected", async () => {
    respond({
      aicli_set_privacy_custom: () => {
        throw { code: "invalid_input", message: "bad regex" };
      },
    });
    renderWithQueryClient(<AicliRoutingSettings onOpenApp={vi.fn()} />);
    fireEvent.click(
      await screen.findByRole("button", { name: "aicli.settings.saveCustom" }),
    );
    expect(
      await screen.findByText("aicli.settings.customInvalid"),
    ).toBeInTheDocument();
  });

  it("warns when credentials fell back to a local file", async () => {
    respond({
      aicli_get_privacy: privacy({ secret_backend: "file_fallback" }),
    });
    renderWithQueryClient(<AicliRoutingSettings onOpenApp={vi.fn()} />);
    expect(
      await screen.findByText("aicli.settings.secretFallback"),
    ).toBeInTheDocument();
  });

  it("does not warn while the keychain is in use", async () => {
    respond();
    renderWithQueryClient(<AicliRoutingSettings onOpenApp={vi.fn()} />);
    await screen.findByRole("switch", {
      name: "aicli.settings.privacyEnabled",
    });
    expect(
      screen.queryByText("aicli.settings.secretFallback"),
    ).not.toBeInTheDocument();
  });

  it("lists taken-over clients with a link to their client page", async () => {
    const onOpenApp = vi.fn();
    respond({
      aicli_takeover_status: [
        { app: "codex", state: "active", taken_over_at: 1 },
        { app: "claude", state: "released", taken_over_at: null },
      ],
    });
    renderWithQueryClient(<AicliRoutingSettings onOpenApp={onOpenApp} />);
    expect(await screen.findByText("Codex")).toBeInTheDocument();
    expect(screen.queryByText("Claude Code")).not.toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", { name: "routingSettings.goTo" }),
    );
    expect(onOpenApp).toHaveBeenCalledWith("codex");
  });
});
