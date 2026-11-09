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
