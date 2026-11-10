import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  AicliPrivacyPage,
  privacyCategoryOf,
} from "@/components/aicli/privacy/AicliPrivacyPage";
import type {
  AicliPrivacyEventPage,
  AicliPrivacySummary,
  AicliPrivacyView,
} from "@/types/aicli";
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
  engine: { masked_values: 4, degraded: 0, engine_blocked: 0, media_passed: 0 },
  custom_words: [],
  custom_rules: [],
  secret_backend: "keychain",
  ...patch,
});

const day = (offset: number) => {
  const d = new Date();
  d.setDate(d.getDate() - offset);
  return d.toISOString().slice(0, 10);
};

const summary = (days = 7): AicliPrivacySummary => ({
  days: Array.from({ length: days }, (_, index) => ({
    day: day(days - 1 - index),
    values: index === days - 1 ? 5812 : 3,
    requests: 1,
  })),
  total_values: 5830,
  distinct_values: 336,
  requests: 1200,
  blocked: 2,
  degraded: 0,
  media_passed: 1,
  categories: [
    { key: "secret", count: 4900 },
    { key: "personal", count: 900 },
    { key: "network", count: 30 },
    { key: "custom", count: 0 },
  ],
  kinds: [{ key: "API_KEY", count: 4900 }],
  apps: [
    { key: "claude", count: 3100 },
    { key: "codex", count: 2700 },
  ],
  range_days: days,
  retention_days: 30,
});

const now = Date.now();
const page: AicliPrivacyEventPage = {
  items: [
    {
      id: 3,
      ts: now,
      app_type: "claude",
      line_id: "main",
      outcome: "masked",
      reason: null,
      value_count: 2,
      distinct_count: 2,
      kinds: [
        { key: "API_KEY", count: 1 },
        { key: "PASSWORD", count: 1 },
      ],
    },
    {
      id: 2,
      ts: now - 60_000,
      app_type: "codex",
      line_id: "global",
      outcome: "blocked",
      reason: "privacy_media_blocked",
      value_count: 0,
      distinct_count: 0,
      kinds: [],
    },
    {
      id: 1,
      ts: now - 3 * 24 * 3600_000,
      app_type: null,
      line_id: null,
      outcome: "masked",
      reason: null,
      value_count: 1,
      distinct_count: 1,
      kinds: [{ key: "EMAIL", count: 1 }],
    },
  ],
  next_before_id: null,
};

function respond(patch: Record<string, unknown> = {}) {
  const responses: Record<string, unknown> = {
    aicli_get_privacy: privacy(),
    aicli_privacy_summary: (args: { days: number }) => summary(args.days),
    aicli_privacy_events: page,
    ...patch,
  };
  invokeMock.mockImplementation(async (command: string, args?: unknown) => {
    const reply = responses[command];
    if (reply instanceof Error) throw reply;
    if (typeof reply === "function")
      return (reply as (a: unknown) => unknown)(args);
    return reply;
  });
}

beforeEach(() => {
  invokeMock.mockReset();
});

describe("privacyCategoryOf", () => {
  it("groups kinds like the backend", () => {
    expect(privacyCategoryOf("API_KEY")).toBe("secret");
    expect(privacyCategoryOf("EMAIL")).toBe("personal");
    expect(privacyCategoryOf("HOSTNAME")).toBe("network");
    expect(privacyCategoryOf("MY_RULE")).toBe("custom");
  });
});

describe("AicliPrivacyPage", () => {
  it("shows what was kept from the upstream: total, distinct values, requests and clients", async () => {
    respond();
    renderWithQueryClient(<AicliPrivacyPage />);
    const hero = await screen.findByTestId("aicli-privacy-hero");
    await waitFor(() =>
      expect(within(hero).getByTestId("aicli-privacy-total")).toHaveAttribute(
        "title",
        (5830).toLocaleString("zh"),
      ),
    );
    expect(
      within(hero).getByTestId("aicli-privacy-distinct"),
    ).toHaveTextContent("336");
    expect(
      within(hero).getByTestId("aicli-privacy-app-claude"),
    ).toHaveTextContent("Claude Code");
    expect(
      within(hero).getByTestId("aicli-privacy-app-codex"),
    ).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("aicli_privacy_summary", {
      days: 7,
    });
  });

  it("switches the range to 30 days", async () => {
    respond();
    renderWithQueryClient(<AicliPrivacyPage />);
    await screen.findByTestId("aicli-privacy-hero");
    fireEvent.click(
      // 分段控件是 aria-pressed 的按钮组
      screen.getAllByRole("button", { name: "aicli.overview.range30" })[0],
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("aicli_privacy_summary", {
        days: 30,
      }),
    );
  });

  it("lists records by day with kinds, outcome and count, and filters by category", async () => {
    respond();
    renderWithQueryClient(<AicliPrivacyPage />);
    const masked = await screen.findByTestId("aicli-privacy-event-3");
    expect(masked).toHaveTextContent("Claude Code");
    // 测试里没有翻译资源，类别名回落为原始类别
    expect(masked).toHaveTextContent("API_KEY");
    expect(masked).toHaveTextContent("×1");
    expect(masked).toHaveTextContent("2");
    const blocked = screen.getByTestId("aicli-privacy-event-2");
    expect(blocked).toHaveTextContent("aicli.privacyPage.outcome.blocked");
    expect(screen.getByTestId("aicli-privacy-event-1")).toHaveTextContent(
      "aicli.privacyPage.otherClient",
    );
    // 今天的两条在同一组
    expect(screen.getAllByText("aicli.overview.today").length).toBeGreaterThan(
      0,
    );

    fireEvent.click(
      screen.getByRole("button", {
        name: /aicli\.privacyPage\.categories\.personal/,
      }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("aicli_privacy_events", {
        days: 7,
        category: "personal",
        beforeId: null,
        limit: 50,
      }),
    );
  });

  it("explains the empty state and the retention", async () => {
    respond({ aicli_privacy_events: { items: [], next_before_id: null } });
    renderWithQueryClient(<AicliPrivacyPage />);
    expect(
      await screen.findByTestId("aicli-privacy-empty"),
    ).toBeInTheDocument();
    expect(screen.getByText("aicli.privacyPage.footer")).toBeInTheDocument();
  });

  it("previews a text without sending it anywhere", async () => {
    respond({
      aicli_privacy_preview: {
        masked: "key {{API_KEY_abcd2345}} ok",
        count: 1,
        kinds: [{ key: "API_KEY", count: 1 }],
      },
    });
    renderWithQueryClient(<AicliPrivacyPage />);
    fireEvent.click(
      await screen.findByRole("button", {
        name: /aicli\.privacyPage\.tryTitle/,
      }),
    );
    fireEvent.change(
      screen.getByRole("textbox", { name: "aicli.privacyPage.tryTitle" }),
      { target: { value: "key sk-secret ok" } },
    );
    fireEvent.click(
      screen.getByRole("button", { name: "aicli.privacyPage.tryRun" }),
    );
    const preview = await screen.findByTestId("aicli-privacy-preview");
    expect(preview).toHaveTextContent("{{API_KEY_abcd2345}}");
    expect(within(preview).getByText("{{API_KEY_abcd2345}}").tagName).toBe(
      "MARK",
    );
    expect(invokeMock).toHaveBeenCalledWith("aicli_privacy_preview", {
      text: "key sk-secret ok",
    });
  });

  it("offers to turn masking on while it is off", async () => {
    respond({
      aicli_get_privacy: privacy({ privacy_enabled: false, engine: null }),
      aicli_set_privacy: undefined,
    });
    renderWithQueryClient(<AicliPrivacyPage />);
    expect(
      await screen.findByText("aicli.privacyPage.offNotice"),
    ).toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", { name: "aicli.privacyPage.enable" }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("aicli_set_privacy", {
        privacyEnabled: true,
        privacyFailureMode: "block",
        allowUnscannedMedia: false,
        privacyDetectCjkNames: null,
      }),
    );
  });

  it("shows a red notice when availability-first mode degraded", async () => {
    respond({
      aicli_get_privacy: privacy({
        privacy_failure_mode: "pass",
        engine: {
          masked_values: 3,
          degraded: 2,
          engine_blocked: 0,
          media_passed: 0,
        },
      }),
    });
    renderWithQueryClient(<AicliPrivacyPage />);
    const notice = await screen.findByText("aicli.privacy.degraded");
    expect(notice.closest(".bg-danger-soft")).not.toBeNull();
  });

  it("clears the records only after confirming", async () => {
    respond({ aicli_privacy_clear_events: undefined });
    renderWithQueryClient(<AicliPrivacyPage />);
    await screen.findByTestId("aicli-privacy-event-3");
    fireEvent.click(
      screen.getByRole("button", { name: "aicli.privacyPage.clear" }),
    );
    expect(
      invokeMock.mock.calls.some(
        ([command]) => command === "aicli_privacy_clear_events",
      ),
    ).toBe(false);
    const dialog = await screen.findByRole("dialog");
    fireEvent.click(
      within(dialog).getByRole("button", { name: "aicli.privacyPage.clear" }),
    );
    await waitFor(() =>
      expect(
        invokeMock.mock.calls.some(
          ([command]) => command === "aicli_privacy_clear_events",
        ),
      ).toBe(true),
    );
  });
});
