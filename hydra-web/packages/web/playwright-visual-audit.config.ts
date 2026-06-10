import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./integration",
  testMatch: "visual-audit.spec.ts",
  outputDir: "./test-results",
  workers: 1,
  reporter: "list",
  use: {
    baseURL: "http://localhost:3000",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: [
    {
      command: "pnpm --filter @hydra/api build && pnpm --filter @hydra/mock-server dev",
      port: 8080,
      reuseExistingServer: !process.env.CI,
      cwd: "../..",
      timeout: 120000,
      // Freeze synthetic-events loop so background emissions don't race
      // visual-audit screenshots. See playwright.config.ts for full rationale.
      env: { MOCK_SYNTHETIC_EVENTS: "0" },
    },
    {
      command:
        "pnpm --filter @hydra/api build && pnpm --filter @hydra/ui build && pnpm --filter @hydra/web dev",
      port: 3000,
      reuseExistingServer: !process.env.CI,
      cwd: "../..",
      timeout: 120000,
    },
  ],
});
