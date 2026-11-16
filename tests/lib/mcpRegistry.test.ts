import { describe, expect, it } from "vitest";
import {
  applyInputs,
  firstMissingInput,
  fromRegistry,
  mergeDiscoverResults,
  needsInput,
  parseRegistryResponse,
  serverKey,
  shortName,
  type McpCatalogEntry,
} from "@/lib/mcpRegistry";
import { mcpCatalog } from "@/config/mcpCatalog";

const featured = (id: string) => {
  const entry = mcpCatalog.find((item) => item.id === id);
  if (!entry) throw new Error(`missing ${id}`);
  return entry;
};

describe("shortName", () => {
  it("drops the namespace and common mcp prefixes / suffixes", () => {
    expect(shortName("io.github.foo/mcp-server-postgres")).toBe("postgres");
    expect(shortName("com.example/weather-mcp")).toBe("weather");
    expect(shortName("io.github.bar/notes")).toBe("notes");
  });
});

describe("serverKey", () => {
  it("normalizes remote URLs", () => {
    expect(serverKey({ type: "http", url: "https://A.example/mcp/" })).toBe(
      "https://a.example/mcp",
    );
  });

  it("uses the command and the first package without its version", () => {
    expect(
      serverKey({
        type: "stdio",
        command: "npx",
        args: ["-y", "@playwright/mcp@latest"],
      }),
    ).toBe("npx @playwright/mcp");
    expect(
      serverKey({
        type: "stdio",
        command: "cmd",
        args: ["/c", "npx", "-y", "@playwright/mcp"],
      }),
    ).toBe("npx @playwright/mcp");
    expect(
      serverKey({ type: "stdio", command: "uvx", args: ["mcp-server-git==1"] }),
    ).toBe("uvx mcp-server-git");
  });
});

describe("fromRegistry", () => {
  it("prefers a streamable-http remote and turns secret headers into inputs", () => {
    const entry = fromRegistry({
      name: "io.github.acme/acme-mcp",
      title: "Acme",
      description: "Acme things",
      repository: { url: "https://github.com/acme/acme-mcp" },
      remotes: [
        { type: "sse", url: "https://acme.dev/sse" },
        {
          type: "streamable-http",
          url: "https://acme.dev/mcp",
          headers: [
            { name: "X-Client", value: "aicli" },
            {
              name: "Authorization",
              value: "Bearer {token}",
              isRequired: true,
              isSecret: true,
            },
          ],
        },
      ],
      packages: [{ registryType: "npm", identifier: "acme-mcp" }],
    })!;
    expect(entry.id).toBe("acme");
    expect(entry.publisher).toBe("acme");
    expect(entry.icon).toBe("https://github.com/acme.png?size=96");
    expect(entry.spec).toEqual({
      type: "http",
      url: "https://acme.dev/mcp",
      headers: { "X-Client": "aicli" },
    });
    expect(entry.runs).toBeUndefined();
    expect(entry.inputs).toEqual([
      expect.objectContaining({
        key: "Authorization",
        where: "header",
        required: true,
        secret: true,
        format: "Bearer {}",
      }),
    ]);
    expect(needsInput(entry)).toBe(true);
  });

  it("skips remotes with URL placeholders and falls back to an npm package", () => {
    const entry = fromRegistry({
      name: "com.example/db",
      remotes: [{ type: "streamable-http", url: "https://{host}/mcp" }],
      packages: [
        {
          registryType: "npm",
          identifier: "@example/db-mcp",
          environmentVariables: [
            { name: "DB_URL", isRequired: true, isSecret: true },
            { name: "LOG_LEVEL", value: "info" },
            { name: "OPTIONAL_THING" },
          ],
          packageArguments: [
            { type: "positional", value: "--stdio" },
            { type: "positional", valueHint: "path", isRequired: true },
          ],
        },
      ],
    })!;
    expect(entry.publisher).toBe("example.com");
    expect(entry.runs).toBe("npx");
    expect(entry.spec.args).toEqual(
      expect.arrayContaining(["-y", "@example/db-mcp", "--stdio"]),
    );
    expect(entry.spec.env).toEqual({ LOG_LEVEL: "info" });
    expect(entry.inputs.map((input) => [input.key, input.where])).toEqual([
      ["arg1", "arg"],
      ["DB_URL", "env"],
    ]);
  });

  it("builds uvx for PyPI and gives up on docker-only or required named args", () => {
    expect(
      fromRegistry({
        name: "io.github.x/py",
        packages: [{ registryType: "pypi", identifier: "py-mcp" }],
      })?.spec,
    ).toEqual({ type: "stdio", command: "uvx", args: ["py-mcp"] });
    expect(
      fromRegistry({
        name: "io.github.x/oci",
        packages: [{ registryType: "oci", identifier: "x/oci" }],
      }),
    ).toBeNull();
    expect(
      fromRegistry({
        name: "io.github.x/named",
        packages: [
          {
            registryType: "npm",
            identifier: "named",
            packageArguments: [
              { type: "named", name: "--token", isRequired: true },
            ],
          },
        ],
      }),
    ).toBeNull();
  });
});

describe("parseRegistryResponse", () => {
  it("drops non-active entries and duplicates", () => {
    const server = {
      name: "io.github.a/one",
      remotes: [{ type: "streamable-http", url: "https://one.dev/mcp" }],
    };
    const result = parseRegistryResponse({
      servers: [
        { server },
        { server: { ...server, name: "io.github.a/one-copy" } },
        {
          server: {
            name: "io.github.a/old",
            remotes: [{ type: "streamable-http", url: "https://old.dev/mcp" }],
          },
          _meta: {
            "io.modelcontextprotocol.registry/official": {
              status: "deprecated",
            },
          },
        },
      ],
    });
    expect(result.map((entry) => entry.registryName)).toEqual([
      "io.github.a/one",
    ]);
  });
});

describe("applyInputs", () => {
  it("fills headers through their template, env vars and args", () => {
    const github = featured("github");
    expect(firstMissingInput(github, {})?.key).toBe("Authorization");
    expect(applyInputs(github, { Authorization: " ghp_1 " }).headers).toEqual({
      Authorization: "Bearer ghp_1",
    });
    // 原条目不被改动
    expect(github.spec.headers).toBeUndefined();

    const brave = featured("brave-search");
    expect(applyInputs(brave, { BRAVE_API_KEY: "k" }).env).toEqual({
      BRAVE_API_KEY: "k",
    });

    const fs = featured("filesystem");
    expect(applyInputs(fs, { dir: "~/code" }).args?.at(-1)).toBe("~/code");
  });

  it("leaves optional inputs out when empty", () => {
    const context7 = featured("context7");
    expect(needsInput(context7)).toBe(false);
    expect(applyInputs(context7, {}).headers).toBeUndefined();
  });
});

describe("mergeDiscoverResults", () => {
  it("keeps featured first, removes registry duplicates and ranks by name", () => {
    const registry: McpCatalogEntry[] = [
      {
        ...featured("deepwiki"),
        source: "registry",
        registryName: "x/deepwiki",
      },
      {
        id: "postgres-tools",
        title: "postgres-tools",
        publisher: "p",
        description: "",
        spec: { type: "http", url: "https://pt.dev/mcp" },
        inputs: [],
        source: "registry",
      },
      {
        id: "postgres",
        title: "postgres",
        publisher: "p",
        description: "",
        spec: { type: "http", url: "https://pg.dev/mcp" },
        inputs: [],
        source: "registry",
      },
    ];
    const merged = mergeDiscoverResults(
      [featured("deepwiki")],
      registry,
      "postgres",
    );
    expect(merged.map((entry) => `${entry.source}:${entry.id}`)).toEqual([
      "featured:deepwiki",
      "registry:postgres",
      "registry:postgres-tools",
    ]);
  });
});

describe("mcpCatalog", () => {
  it("has unique ids and unique servers", () => {
    const ids = mcpCatalog.map((entry) => entry.id);
    expect(new Set(ids).size).toBe(ids.length);
    const keys = mcpCatalog.map((entry) => serverKey(entry.spec));
    expect(new Set(keys).size).toBe(keys.length);
  });
});
