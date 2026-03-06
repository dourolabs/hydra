import { test as base, expect } from "@playwright/test";
import type { Page } from "@playwright/test";

type AuthFixtures = {
  authenticatedPage: Page;
};

export const test = base.extend<AuthFixtures>({
  authenticatedPage: async ({ page }, use) => {
    // Reset mock server state to ensure clean seed data
    await fetch("http://localhost:8080/v1/dev/reset", {
      method: "POST",
      headers: { Authorization: "Bearer dev-token-12345" },
    });
    await page.goto("/login");
    await page.waitForSelector('[data-testid="token-input"]');
    await page.fill('[data-testid="token-input"]', "dev-token-12345");
    await page.click('[data-testid="login-button"]');
    await page.waitForURL((url) => !url.pathname.startsWith("/login"), {
      timeout: 10000,
    });
    await use(page);
  },
});

export { expect };
