import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * B-07 只检查了语言文件；组件里写死的旧品牌名（侧栏左上角、关于页、托盘面板）
 * 漏掉了。这里扫描 src 下的组件：界面文字不得是旧品牌名，也不得再引用旧 logo。
 * 注释、翻译缺失时的 defaultValue 兜底文案、合作方链接参数不在此列。
 */
function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return sources(path);
    return /\.tsx$/.test(name) ? [path] : [];
  });
}

describe("product brand in components", () => {
  const files = sources(join(process.cwd(), "src"));

  it("scans the component sources", () => {
    expect(files.length).toBeGreaterThan(100);
  });

  it("never renders the legacy product name as UI text", () => {
    const offenders = files.flatMap((file) => {
      const text = readFileSync(file, "utf8");
      // JSX 文本节点：>CC Switch< 或 >CC Switch v{version}
      return /> *\n? *CC Switch(?: v\{|\s*<)/.test(text) ? [file] : [];
    });
    expect(offenders).toEqual([]);
  });

  it("never imports the legacy logo", () => {
    const offenders = files.filter((file) =>
      /assets\/icons\/logo\.svg/.test(readFileSync(file, "utf8")),
    );
    expect(offenders).toEqual([]);
  });
});

// ---- 全库扫描后补充（2026-11-09）：后端字符串、外部链接、安装包模板 ----

const ROOT = process.cwd();
const LEGACY_LINK = /farion1231\/cc-switch|ccswitch\.io/;

function files(dir: string, pattern: RegExp): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return files(path, pattern);
    return pattern.test(name) ? [path] : [];
  });
}

/** 去掉整行注释（// … 与 /* … *\/ 块中以 * 开头的行），只看代码。 */
function codeLines(text: string): string[] {
  return text
    .split("\n")
    .filter((line) => !/^\s*(\/\/|\/\*|\*|\{\/\*)/.test(line));
}

/** Rust 文件里测试模块之前的部分（运行时代码）。 */
function rustRuntime(text: string): string {
  const match = /\n#\[cfg\(test\)\]\n(pub(\(crate\))? )?mod \w+/.exec(text);
  return match ? text.slice(0, match.index) : text;
}

const RUST_LITERAL = /"(?:[^"\\\n]|\\.)*"/g;

/** 允许出现旧品牌名的运行时字符串：说的就是 cc-switch 本身，或兼容旧数据。 */
const RUST_ALLOWED = [
  // 接管冲突：客户端配置里有旧 CC Switch 的代理设置（指的就是那个应用）
  "CC Switch is managing this client; confirmation is required",
  // 导入改名前导出的 SQL 备份
  "-- CC Switch SQLite 导出",
  // 对话历史里改名前写入的工具出错标记，识别时仍接受
  "[cc-switch:tool-result-error]",
];

describe("product brand across the app", () => {
  it("links to AICLI, not the upstream repository or website (frontend)", () => {
    const offenders = files(join(ROOT, "src"), /\.(ts|tsx|html)$/)
      .filter((file) => !/Presets?\.ts$|presets\//.test(file))
      .filter((file) =>
        codeLines(readFileSync(file, "utf8")).some((line) =>
          LEGACY_LINK.test(line),
        ),
      );
    expect(offenders).toEqual([]);
  });

  it("keeps the legacy name out of backend runtime strings", () => {
    const offenders: string[] = [];
    for (const file of files(join(ROOT, "src-tauri", "src"), /\.rs$/)) {
      if (/(^|[/_])tests?\.rs$/.test(file)) continue;
      const runtime = codeLines(rustRuntime(readFileSync(file, "utf8"))).join(
        "\n",
      );
      for (const literal of runtime.match(RUST_LITERAL) ?? []) {
        const value = literal.slice(1, -1);
        if (RUST_ALLOWED.some((allowed) => value.includes(allowed))) continue;
        if (
          /CC Switch|CC-Switch|\[cc-switch:/.test(value) ||
          LEGACY_LINK.test(value)
        ) {
          offenders.push(`${file}: ${value.slice(0, 80)}`);
        }
      }
    }
    expect(offenders).toEqual([]);
  });

  it("never closes cc-switch.exe from the Windows installer", () => {
    const wix = readFileSync(
      join(ROOT, "src-tauri", "wix", "per-user-main.wxs"),
      "utf8",
    );
    expect(wix).not.toMatch(/cc-switch\.exe/i);
    expect(wix).toMatch(/Target="aicli-client\.exe"/);
  });

  it("names the product in the macOS bundle strings", () => {
    const plist = readFileSync(join(ROOT, "src-tauri", "Info.plist"), "utf8");
    expect(plist).not.toMatch(/CC Switch/);
  });
});
