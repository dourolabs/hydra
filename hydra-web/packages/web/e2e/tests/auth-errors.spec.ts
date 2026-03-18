import { test, expect } from "@playwright/test";

test.describe("Auth Errors @auth:invalid-token @auth:logout", () => {
  test("shows error when logging in with invalid token @auth:invalid-token", async ({
    page,
  }) => {
    // Intercept the login API to simulate an authentication failure
    await page.route("**/auth/login", (route) => {
      route.fulfill({
        status: 401,
        contentType: "application/json",
        body: JSON.stringify({ error: "Invalid token" }),
      });
    });

    await page.goto("/login");
    await page.fill('[data-testid="token-input"]', "invalid-token-99999");
    await page.click('[data-testid="login-button"]');

    // Should show an error message and remain on login page
    await expect(page.getByText(/error|invalid|failed/i)).toBeVisible({
      timeout: 10000,
    });
    await expect(page).toHaveURL(/\/login/);
  });

  test("can log out and is redirected to login @auth:logout", async ({
    page,
  }) => {
    // First, log in
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

    // Now click the logout button in the sidebar
    await page.getByRole("button", { name: /logout/i }).click();

    // Should be redirected to login page
    await expect(page).toHaveURL(/\/login/, { timeout: 10000 });
  });
});
