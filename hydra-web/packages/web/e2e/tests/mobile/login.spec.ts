import { test, expect } from "@playwright/test";

test.describe("Mobile Login @mobile:login", () => {
  test("shows GitHub login button as primary action @mobile:login", async ({
    page,
  }) => {
    await page.goto("/login");
    await expect(
      page.locator('[data-testid="github-login-button"]')
    ).toBeVisible();
    await expect(page.getByText("Sign in with token")).toBeVisible();
  });

  test("device flow works on mobile @mobile:login", async ({ page }) => {
    await page.goto("/login");
    await page.click('[data-testid="github-login-button"]');
    // Device flow starts — user code should appear
    await expect(page.getByText("MOCK-1234")).toBeVisible();
    await expect(page.getByText("Waiting for authorization")).toBeVisible();
    // Mock poll returns complete instantly — should redirect to dashboard
    await expect(page).not.toHaveURL(/\/login/, { timeout: 10000 });
  });

  test("token fallback works on mobile @mobile:login", async ({ page }) => {
    await page.goto("/login");
    await page.click("text=Sign in with token");
    await expect(page.locator('[data-testid="token-input"]')).toBeVisible();
    await page.fill('[data-testid="token-input"]', "dev-token-12345");
    await expect(page.locator('[data-testid="login-button"]')).toBeEnabled();
    await page.click('[data-testid="login-button"]');
    await expect(page).not.toHaveURL(/\/login/, { timeout: 10000 });
  });
});
