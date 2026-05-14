import { test, expect } from "@playwright/test";

test.describe("Auth Errors @auth:logout", () => {
  test("can log out and is redirected to login @auth:logout", async ({
    page,
  }) => {
    // First, log in via the GitHub device flow (mock server completes instantly)
    await fetch("http://localhost:8080/v1/dev/reset", {
      method: "POST",
      headers: { Authorization: "Bearer dev-token-12345" },
    });
    await page.goto("/login");
    await page.click('[data-testid="github-login-button"]');
    await page.waitForURL((url) => !url.pathname.startsWith("/login"), {
      timeout: 10000,
    });

    // Now click the logout button in the sidebar
    await page.getByRole("button", { name: /logout/i }).click();

    // Should be redirected to login page
    await expect(page).toHaveURL(/\/login/, { timeout: 10000 });
  });
});
