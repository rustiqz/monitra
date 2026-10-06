import { defineConfig } from "vitest/config";

// Pure-logic tests only (no DOM): `window` is stubbed per test, so Node's
// environment is enough and the suite needs no jsdom dependency.
export default defineConfig({
  test: { environment: "node", include: ["src/**/*.test.ts"] },
});
