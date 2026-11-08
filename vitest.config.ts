import path from "node:path";
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  test: {
    include: ["{src,tests}/**/*.{test,spec}.{ts,tsx,js,jsx}"],
    environment: "jsdom",
    setupFiles: ["./tests/setupGlobals.ts", "./tests/setupTests.ts"],
    globals: true,
    // Windows/jsdom runs several times slower than the Linux CI runners;
    // the default 5s timeout caused mass flaky timeouts there (see
    // docs/aicli-handover/E-GATE.md). Assertions are unaffected.
    testTimeout: 60000,
    hookTimeout: 30000,
    // 24 logical cores × default worker count oversubscribes the machine and
    // slows heavy jsdom suites several-fold on Windows; cap it there only so
    // CI runners are unaffected.
    maxWorkers: process.platform === "win32" ? 8 : undefined,
    minWorkers: process.platform === "win32" ? 1 : undefined,
    coverage: {
      reporter: ["text", "lcov"],
    },
  },
});
