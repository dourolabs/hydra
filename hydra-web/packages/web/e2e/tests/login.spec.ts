import { test, expect } from "@playwright/test";

test.describe("Login @auth:login @auth:redirect", () => {
  test("shows login form with token input and login button @auth:login", async ({
    page,
  }) => {
    await page.goto("/login");
    await expect(page.locator('[data-testid="token-input"]')).toBeVisible();
    await expect(page.locator('[data-testid="login-button"]')).toBeVisible();
    await expect(page.locator('[data-testid="login-button"]')).toBeDisabled();
  });

  test("login button is disabled when token is empty @auth:login", async ({ page }) => {
    await page.goto("/login");
    await expect(page.locator('[data-testid="login-button"]')).toBeDisabled();
  });

  test("login button is enabled when token is entered @auth:login", async ({ page }) => {
    await page.goto("/login");
    await page.fill('[data-testid="token-input"]', "dev-token-12345");
    await expect(page.locator('[data-testid="login-button"]')).toBeEnabled();
  });

  test("successful login redirects to dashboard @auth:login", async ({ page }) => {
    await page.goto("/login");
    await page.fill('[data-testid="token-input"]', "dev-token-12345");
    await page.click('[data-testid="login-button"]');
    // After login, the app should redirect away from /login
    await expect(page).not.toHaveURL(/\/login/);
  });

  test("redirects unauthenticated user to login @auth:redirect", async ({ page }) => {
    await page.goto("/");
    await expect(page).toHaveURL(/\/login/);
  });
});
