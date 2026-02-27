import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e/tests",
  outputDir: "./test-results",
  fullyParallel: true,
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
    },
  ],
  webServer: [
    {
      command: "pnpm --filter @metis/mock-server dev",
      port: 8080,
      reuseExistingServer: true,
      cwd: "../..",
    },
    {
      command:
        "METIS_SERVER_URL=http://localhost:8080 COOKIE_SECURE=false pnpm --filter @metis/web dev:server",
      port: 4000,
      reuseExistingServer: true,
      cwd: "../..",
    },
    {
      command:
        "pnpm --filter @metis/api build && pnpm --filter @metis/ui build && pnpm --filter @metis/web dev",
      port: 3000,
      reuseExistingServer: true,
      cwd: "../..",
      timeout: 120000,
    },
  ],
});
