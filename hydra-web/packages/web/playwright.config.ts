import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e/tests",
  outputDir: "./test-results",
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  workers: 1,
  reporter: "list",
  use: {
    baseURL: "http://localhost:3000",
    screenshot: "only-on-failure",
    trace: "on-first-retry",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
      testIgnore: /\/mobile\//,
    },
    {
      name: "Mobile Chrome",
      testDir: "./e2e/tests/mobile",
      use: {
        ...devices["Desktop Chrome"],
        viewport: { width: 375, height: 812 },
        hasTouch: true,
      },
    },
  ],
  webServer: [
    {
      command: "pnpm --filter @hydra/mock-server dev",
      port: 8080,
      reuseExistingServer: !process.env.CI,
      cwd: "../..",
      // Freeze the synthetic-events loop during e2e runs so background
      // `tool_use` / `assistant_message` emissions on running sessions don't
      // race against assertions about session-event tails (e.g. the
      // `@chat:activity-status` spec). The loop's only purpose is dev-UI
      // fluidity; tests need deterministic event logs.
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
