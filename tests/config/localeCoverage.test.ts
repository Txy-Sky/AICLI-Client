import { describe, expect, it } from "vitest";
import tr from "@/i18n/locales/tr.json";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import zhTW from "@/i18n/locales/zh-TW.json";
import zh from "@/i18n/locales/zh.json";

type TranslationTree = Record<string, unknown>;

function flattenStrings(
  value: unknown,
  path: string[] = [],
  result = new Map<string, string>(),
): Map<string, string> {
  if (typeof value === "string") {
    result.set(path.join("."), value);
  } else if (typeof value === "object" && value !== null) {
    for (const [key, child] of Object.entries(value)) {
      flattenStrings(child, [...path, key], result);
    }
  }
  return result;
}

function interpolationVariables(value: string): string[] {
  return Array.from(
    value.matchAll(/\{\{\s*([^}]+?)\s*\}\}/g),
    ([, name]) => name,
  ).sort();
}

const reference = flattenStrings(en);
const piKeysOutsideNamespace = new Set([
  "apps.pi",
  "sessionManager.piDiscoveryUnavailable",
  "sessionManager.piRelativeSessionDir",
  "settings.browsePlaceholderPi",
  "settings.piConfigDir",
  "settings.piConfigDirDescription",
]);
const piReference = new Map(
  [...reference].filter(
    ([key]) => key.startsWith("pi.") || piKeysOutsideNamespace.has(key),
  ),
);
const piProductReferences = new Map(
  [...reference].filter(([, value]) => /\bPi\b/.test(value)),
);
const locales = [
  ["zh", zh],
  ["ja", ja],
  ["zh-TW", zhTW],
] as const;

describe("locale coverage", () => {
  it.each(locales)("covers every Pi translation key in %s", (_name, tree) => {
    const translations = flattenStrings(tree as TranslationTree);
    const missing = [...piReference.keys()].filter(
      (key) => !translations.has(key),
    );

    expect(missing).toEqual([]);
  });

  it.each(locales)(
    "preserves every Pi interpolation variable in %s",
    (_name, tree) => {
      const translations = flattenStrings(tree as TranslationTree);
      const mismatched = [...piReference].flatMap(([key, expected]) => {
        const actual = translations.get(key);
        return actual !== undefined &&
          interpolationVariables(actual).join("\0") !==
            interpolationVariables(expected).join("\0")
          ? [key]
          : [];
      });

      expect(mismatched).toEqual([]);
    },
  );

  it.each(locales)(
    "preserves explicit Pi product mentions in %s",
    (_name, tree) => {
      const translations = flattenStrings(tree as TranslationTree);
      const missingMentions = [...piProductReferences.keys()].filter((key) => {
        const actual = translations.get(key);
        return actual === undefined || !/\bPi\b/.test(actual);
      });

      expect(missingMentions).toEqual([]);
    },
  );
});

// Turkish is a community translation: keys it does not have yet fall back to
// English, so new English strings must never fail CI here. These checks only
// look at strings that already exist in tr.json.
describe("Turkish locale", () => {
  it("keeps every existing translation non-empty", () => {
    expect(
      [...flattenStrings(tr)].filter(
        ([key, value]) => reference.get(key)?.trim() && !value.trim(),
      ),
    ).toEqual([]);
  });

  it("preserves interpolation variables in existing translations", () => {
    expect(
      [...flattenStrings(tr)]
        .filter(([key, value]) => {
          const expected = reference.get(key);
          return (
            expected !== undefined &&
            interpolationVariables(value).join("\0") !==
              interpolationVariables(expected).join("\0")
          );
        })
        .map(([key]) => key),
    ).toEqual([]);
  });

  it("preserves protocol fields, configuration literals, and commands", () => {
    const translations = flattenStrings(tr);
    const exactLiterals = [
      "openclaw.providerKeyPlaceholder",
      "hermes.form.providerKeyPlaceholder",
      "mcp.envPlaceholder",
      "mcp.wizard.envPlaceholder",
      "skills.searchSource.skillssh",
    ];

    for (const key of exactLiterals) {
      expect(translations.get(key), key).toBe(reference.get(key));
    }

    // G-01：opencode / grokBuild / codexConfig / providerPreset / usageScript 的
    // 字面量随供应商表单与用量脚本一起删除；G-04 又移除了 settings.s3Sync
    // （WebDAV / S3 云同步）命名空间，协议字面量随之删除
    // 2026-11 改版：命令从正文挪到确认框的补充小字里，仍须原样保留
    expect(translations.get("hermes.webui.launchConfirmHint")).toContain(
      "pip install hermes-agent[web]",
    );
    expect(
      translations.get("settings.manualUninstallBunLauncherHint"),
    ).toContain("OpenClaw Bun launcher");
    expect(translations.get("prompts.contentPlaceholderHermes")).toMatch(
      /^# SOUL\.md\n/,
    );
  });
});
