import { test, expect } from "@playwright/test";

test.describe("Login @auth:login @auth:redirect", () => {
  test("shows GitHub login button as primary action @auth:login", async ({
    page,
  }) => {
    await page.goto("/login");
    await expect(
      page.locator('[data-testid="github-login-button"]')
    ).toBeVisible();
    await expect(page.getByText("Sign in with token")).toBeVisible();
  });

  test("device flow: click GitHub login, see user code, completes and redirects @auth:login", async ({
    page,
  }) => {
    await page.goto("/login");
    await page.click('[data-testid="github-login-button"]');
    // Device flow starts — user code should appear
    await expect(page.getByText("MOCK-1234")).toBeVisible();
    await expect(page.getByText("Waiting for authorization")).toBeVisible();
    // Mock poll returns complete instantly — should redirect to dashboard
    await expect(page).not.toHaveURL(/\/login/, { timeout: 10000 });
  });

  test("token fallback: click switch link, enter token, login @auth:login", async ({
    page,
  }) => {
    await page.goto("/login");
    await page.click("text=Sign in with token");
    await expect(page.locator('[data-testid="token-input"]')).toBeVisible();
    await expect(page.locator('[data-testid="login-button"]')).toBeDisabled();
    await page.fill('[data-testid="token-input"]', "dev-token-12345");
    await expect(page.locator('[data-testid="login-button"]')).toBeEnabled();
    await page.click('[data-testid="login-button"]');
    await expect(page).not.toHaveURL(/\/login/, { timeout: 10000 });
  });

  test("redirects unauthenticated user to login @auth:redirect", async ({
    page,
  }) => {
    await page.goto("/");
    await expect(page).toHaveURL(/\/login/);
  });
});
