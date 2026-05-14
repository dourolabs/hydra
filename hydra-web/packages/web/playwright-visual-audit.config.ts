import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
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
      command: "pnpm --filter @hydra/mock-server dev",
      port: 8080,
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
