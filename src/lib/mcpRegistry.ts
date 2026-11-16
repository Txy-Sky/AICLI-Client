import type { McpServerSpec } from "@/types";
import { createNpxCommand } from "@/config/mcpPresets";

/**
 * MCP 发现栏的条目模型，以及官方 MCP Registry（registry.modelcontextprotocol.io）
 * 的条目转换成可直接写入的配置。规则参考 magpie（internal/library/market.go、
 * market_launch.go）：远程优先，其次 npm（npx）/ PyPI（uvx）；docker 镜像暂不支持。
 */

export type McpInputWhere = "env" | "header" | "arg";

/** 添加时要用户填的一项（密钥、令牌、目录……） */
export interface McpCatalogInput {
  /** 环境变量名 / 请求头名；arg 类为内部键（arg0、dir…） */
  key: string;
  where: McpInputWhere;
  /** 精选条目用翻译键（apiKey / token / folder），Registry 条目直接是原文 */
  labelKey?: "apiKey" | "token" | "folder";
  label?: string;
  /** Registry 给的说明原文 */
  description?: string;
  /** 去哪里拿（显示成链接），如 context7.com/dashboard */
  obtain?: string;
  /** 可选项为什么值得填 */
  optionalNote?: "rateLimit";
  placeholder?: string;
  secret?: boolean;
  required?: boolean;
  /** 请求头的值模板，{} 换成填的值，如 "Bearer {}"；缺省就是值本身 */
  format?: string;
}

export interface McpCatalogEntry {
  /** 写进配置的服务器名（键） */
  id: string;
  title: string;
  publisher: string;
  icon?: string;
  homepage?: string;
  /** 英文原文；精选条目另有翻译（mcpCatalog.<id>） */
  description: string;
  spec: McpServerSpec;
  /** 本地运行方式：npx / uvx；远程条目没有 */
  runs?: "npx" | "uvx";
  /** 远程服务第一次用时要在浏览器里登录（OAuth） */
  signIn?: boolean;
  inputs: McpCatalogInput[];
  source: "featured" | "registry";
  /** Registry 里的完整名字（io.github.foo/bar） */
  registryName?: string;
  repository?: string;
}

// ─── 工具 ──────────────────────────────────────────────────────────────

export function githubOwnerOf(url: string | undefined): string | undefined {
  const match = url?.match(/^https?:\/\/(?:www\.)?github\.com\/([^/?#]+)/i);
  return match?.[1];
}

export function githubAvatarUrl(owner: string | undefined) {
  const name = owner?.trim();
  if (!name || !/^[A-Za-z0-9][A-Za-z0-9-]{0,38}$/.test(name)) return undefined;
  return `https://github.com/${name}.png?size=96`;
}

/** 有没有必须填的项：有就点「添加…」先弹表单 */
export function needsInput(entry: McpCatalogEntry): boolean {
  return entry.inputs.some((input) => input.required);
}

/** 远程地址 / 本地命令 + 第一个非选项参数（去掉版本号）：判重、判断「已添加」用 */
export function serverKey(spec: McpServerSpec | undefined): string {
  if (!spec) return "";
  if (typeof spec.url === "string" && spec.url.trim()) {
    return spec.url.trim().toLowerCase().replace(/\/+$/, "");
  }
  let command = typeof spec.command === "string" ? spec.command : "";
  let args = Array.isArray(spec.args) ? spec.args.map(String) : [];
  // Windows 上 npx 包在 cmd /c 里
  if (command.toLowerCase() === "cmd" && args[0]?.toLowerCase() === "/c") {
    command = args[1] ?? "";
    args = args.slice(2);
  }
  const exe = command
    .split(/[/\\]/)
    .pop()!
    .replace(/\.(cmd|exe|bat)$/i, "")
    .toLowerCase();
  if (!exe) return "";
  const first = args.find((arg) => arg && !arg.startsWith("-")) ?? "";
  const bare = first
    .replace(/(?!^)@[^/@]*$/, "") // npm：@scope/pkg@1.2.3 → @scope/pkg
    .replace(/(==|>=|~=).*$/, ""); // PyPI：pkg==1.0 → pkg
  return `${exe} ${bare}`.trim().toLowerCase();
}

/** 第一个必填却没填的项 */
export function firstMissingInput(
  entry: McpCatalogEntry,
  values: Record<string, string>,
): McpCatalogInput | undefined {
  return entry.inputs.find(
    (input) => input.required && !(values[input.key] ?? "").trim(),
  );
}

/** 把填的值放进配置：env 进环境变量，header 按模板进请求头，arg 追加到参数 */
export function applyInputs(
  entry: McpCatalogEntry,
  values: Record<string, string>,
): McpServerSpec {
  const spec: McpServerSpec = JSON.parse(JSON.stringify(entry.spec));
  for (const input of entry.inputs) {
    const value = (values[input.key] ?? "").trim();
    if (!value) continue;
    if (input.where === "env") {
      spec.env = { ...(spec.env ?? {}), [input.key]: value };
    } else if (input.where === "header") {
      spec.headers = {
        ...(spec.headers ?? {}),
        [input.key]: (input.format ?? "{}").replace("{}", value),
      };
    } else {
      spec.args = [...(spec.args ?? []), value];
    }
  }
  return spec;
}

// ─── Registry ─────────────────────────────────────────────────────────

interface RegistryVar {
  name?: string;
  value?: string;
  default?: string;
  description?: string;
  isRequired?: boolean;
  isSecret?: boolean;
}

interface RegistryArg {
  type?: string;
  name?: string;
  value?: string;
  default?: string;
  valueHint?: string;
  description?: string;
  isRequired?: boolean;
}

interface RegistryPackage {
  registryType?: string;
  identifier?: string;
  runtimeHint?: string;
  transport?: { type?: string };
  environmentVariables?: RegistryVar[];
  packageArguments?: RegistryArg[];
}

export interface RegistryServer {
  name?: string;
  title?: string;
  description?: string;
  websiteUrl?: string;
  repository?: { url?: string };
  icons?: Array<{ src?: string }>;
  remotes?: Array<{ type?: string; url?: string; headers?: RegistryVar[] }>;
  packages?: RegistryPackage[];
}

export interface RegistryResponse {
  servers?: Array<{
    server?: RegistryServer;
    _meta?: Record<string, { status?: string } | undefined>;
  }>;
}

const OFFICIAL_META = "io.modelcontextprotocol.registry/official";

/** io.github.foo/mcp-server-bar → bar */
export function shortName(name: string): string {
  let base = (name.split("/").pop() ?? name).toLowerCase();
  for (const prefix of ["mcp-server-", "server-", "mcp-"]) {
    if (base.startsWith(prefix) && base.length > prefix.length) {
      base = base.slice(prefix.length);
      break;
    }
  }
  for (const suffix of ["-mcp-server", "-mcp", "-server", "_mcp", ".mcp"]) {
    if (base.endsWith(suffix) && base.length > suffix.length) {
      base = base.slice(0, -suffix.length);
      break;
    }
  }
  return base.replace(/[^a-z0-9._-]+/g, "-").replace(/^-+|-+$/g, "") || "mcp";
}

function publisherOf(server: RegistryServer): string {
  const owner = githubOwnerOf(server.repository?.url);
  if (owner) return owner;
  const name = server.name ?? "";
  const ns = name.split("/")[0] ?? "";
  const gh = ns.match(/^io\.github\.(.+)$/i);
  if (gh) return gh[1];
  return ns.split(".").reverse().join(".");
}

const hasPlaceholder = (value: string | undefined) =>
  /\{[^}]*\}/.test(value ?? "");

function remoteOf(server: RegistryServer) {
  for (const type of ["streamable-http", "sse"]) {
    const remote = server.remotes?.find(
      (item) =>
        item.type === type &&
        typeof item.url === "string" &&
        item.url.startsWith("https://") &&
        !item.url.includes("{"),
    );
    if (!remote) continue;
    const headers: Record<string, string> = {};
    const inputs: McpCatalogInput[] = [];
    for (const header of remote.headers ?? []) {
      if (!header.name) continue;
      if (header.value && !hasPlaceholder(header.value)) {
        headers[header.name] = header.value;
      } else if (header.isRequired || header.isSecret) {
        inputs.push({
          key: header.name,
          where: "header",
          label: header.name,
          description: header.description,
          secret: header.isSecret,
          required: header.isRequired,
          format: header.value
            ? header.value.replace(/\{[^}]*\}/g, "{}")
            : undefined,
        });
      }
    }
    const spec: McpServerSpec = {
      type: type === "sse" ? "sse" : "http",
      url: remote.url,
    };
    if (Object.keys(headers).length) spec.headers = headers;
    return { spec, inputs };
  }
  return null;
}

function packageOf(server: RegistryServer) {
  for (const registryType of ["npm", "pypi"] as const) {
    const pkg = server.packages?.find(
      (item) =>
        item.registryType === registryType &&
        item.identifier &&
        (item.transport?.type ?? "stdio") === "stdio",
    );
    if (!pkg?.identifier) continue;
    const runs: "npx" | "uvx" = registryType === "npm" ? "npx" : "uvx";
    if (pkg.runtimeHint && pkg.runtimeHint !== runs) continue;

    const inputs: McpCatalogInput[] = [];
    const extra: string[] = [];
    let unusable = false;
    (pkg.packageArguments ?? []).forEach((arg, index) => {
      const fixed = arg.value ?? arg.default;
      if (arg.type === "named") {
        if (!arg.name) return;
        if (fixed !== undefined && !hasPlaceholder(fixed)) {
          extra.push(arg.name, fixed);
        } else if (arg.isRequired) {
          // 带名字的必填参数没法放进「追加到末尾」的输入里
          unusable = true;
        }
        return;
      }
      if (fixed !== undefined && !hasPlaceholder(fixed)) {
        extra.push(fixed);
      } else if (arg.isRequired) {
        inputs.push({
          key: `arg${index}`,
          where: "arg",
          label: arg.valueHint || arg.name || `arg${index}`,
          description: arg.description,
          required: true,
        });
      }
    });
    if (unusable) continue;

    const env: Record<string, string> = {};
    for (const item of pkg.environmentVariables ?? []) {
      if (!item.name) continue;
      if (item.value && !hasPlaceholder(item.value)) {
        env[item.name] = item.value;
      } else if (item.isRequired || item.isSecret) {
        inputs.push({
          key: item.name,
          where: "env",
          label: item.name,
          description: item.description,
          secret: item.isSecret,
          required: item.isRequired,
        });
      }
    }

    const spec: McpServerSpec =
      runs === "npx"
        ? {
            type: "stdio",
            ...createNpxCommand(pkg.identifier, ["-y"]),
          }
        : { type: "stdio", command: "uvx", args: [pkg.identifier] };
    spec.args = [...(spec.args ?? []), ...extra];
    if (Object.keys(env).length) spec.env = env;
    return { spec, inputs, runs };
  }
  return null;
}

/** 一条 Registry 记录 → 发现栏条目；装不了（docker、带占位的地址……）返回 null */
export function fromRegistry(server: RegistryServer): McpCatalogEntry | null {
  if (!server.name) return null;
  const remote = remoteOf(server);
  const local = remote ? null : packageOf(server);
  const picked = remote ?? local;
  if (!picked) return null;
  const icon =
    server.icons?.find((item) => item.src?.startsWith("https://"))?.src ??
    githubAvatarUrl(githubOwnerOf(server.repository?.url));
  return {
    id: shortName(server.name),
    title: server.title?.trim() || shortName(server.name),
    publisher: publisherOf(server),
    icon,
    homepage: server.websiteUrl || server.repository?.url || undefined,
    description: server.description?.trim() ?? "",
    spec: picked.spec,
    runs: local?.runs,
    inputs: picked.inputs,
    source: "registry",
    registryName: server.name,
    repository: server.repository?.url,
  };
}

/** 整页响应 → 条目（跳过非 active 的、装不了的；同一服务只留第一条） */
export function parseRegistryResponse(
  response: RegistryResponse | null | undefined,
): McpCatalogEntry[] {
  const out: McpCatalogEntry[] = [];
  const seen = new Set<string>();
  for (const item of response?.servers ?? []) {
    const status = item._meta?.[OFFICIAL_META]?.status;
    if (status && status !== "active") continue;
    if (!item.server) continue;
    const entry = fromRegistry(item.server);
    if (!entry) continue;
    const key = serverKey(entry.spec);
    if (!key || seen.has(key)) continue;
    seen.add(key);
    out.push(entry);
  }
  return out;
}

/** 搜索结果排序分：名字越像查询词越靠前，有图标 / 仓库的略靠前 */
export function rankEntry(entry: McpCatalogEntry, query: string): number {
  const q = query.trim().toLowerCase();
  let score = 0;
  const id = entry.id.toLowerCase();
  const title = entry.title.toLowerCase();
  if (q && (id === q || title === q)) score += 100;
  else if (q && id.includes(q)) score += 40;
  if (q && title.includes(q)) score += 20;
  if (q && entry.description.toLowerCase().includes(q)) score += 10;
  if (entry.icon) score += 8;
  if (entry.repository) score += 5;
  if (entry.registryName?.startsWith("ai.smithery/")) score -= 60;
  return score;
}

/** 精选在前（已按查询词过滤），Registry 结果按分排序、和精选重复的去掉，最多 limit 条 */
export function mergeDiscoverResults(
  featured: McpCatalogEntry[],
  registry: McpCatalogEntry[],
  query: string,
  limit = 100,
): McpCatalogEntry[] {
  const seen = new Set(featured.map((entry) => serverKey(entry.spec)));
  const rest = registry
    .filter((entry) => {
      const key = serverKey(entry.spec);
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    })
    .map((entry, index) => ({ entry, index, score: rankEntry(entry, query) }))
    .sort((a, b) => b.score - a.score || a.index - b.index)
    .map((item) => item.entry);
  return [...featured, ...rest].slice(0, limit);
}
