import { defineConfig } from "vitest/config";

export default defineConfig({
  resolve: {
    conditions: ["source"],
  },
  test: {
    globals: true,
    environment: "jsdom",
    include: ["packages/*/src/**/*.test.{ts,tsx}"],
    passWithNoTests: true,
  },
});
