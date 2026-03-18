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
      reuseExistingServer: true,
      cwd: "../..",
    },
    {
      command:
        "HYDRA_SERVER_URL=http://localhost:8080 COOKIE_SECURE=false pnpm --filter @hydra/web dev:server",
      port: 4000,
      reuseExistingServer: true,
      cwd: "../..",
    },
    {
      command:
        "pnpm --filter @hydra/api build && pnpm --filter @hydra/ui build && pnpm --filter @hydra/web dev",
      port: 3000,
      reuseExistingServer: true,
      cwd: "../..",
      timeout: 120000,
    },
  ],
});
