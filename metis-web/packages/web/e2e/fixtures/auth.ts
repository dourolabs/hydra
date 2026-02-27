import { test as base, expect } from "@playwright/test";
import type { Page } from "@playwright/test";

type AuthFixtures = {
  authenticatedPage: Page;
};

export const test = base.extend<AuthFixtures>({
  authenticatedPage: async ({ page }, use) => {
    await page.goto("/login");
    await page.fill('[data-testid="token-input"]', "dev-token-12345");
    await page.click('[data-testid="login-button"]');
    // Wait until we leave the login page
    await page.waitForFunction(() => !window.location.pathname.startsWith("/login"));
    await use(page);
  },
});

export { expect };
