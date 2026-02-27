import { test, expect } from "../fixtures/auth";

test.describe("Navigation", () => {
  test("sidebar links navigate to correct pages", async ({
    authenticatedPage: page,
  }) => {
    // Navigate to Issues via sidebar (icon-only links, use href selector)
    await page.locator('nav a[href="/issues"]').click();
    await expect(page).toHaveURL(/\/issues/);

    // Navigate to Patches via sidebar
    await page.locator('nav a[href="/patches"]').click();
    await expect(page).toHaveURL(/\/patches/);

    // Navigate to Documents via sidebar
    await page.locator('nav a[href="/documents"]').click();
    await expect(page).toHaveURL(/\/documents/);

    // Navigate to Settings via sidebar
    await page.locator('nav a[href="/settings"]').click();
    await expect(page).toHaveURL(/\/settings/);

    // Navigate to Dashboard via sidebar
    await page.getByTestId('nav-dashboard').click();
    await expect(page).toHaveURL(/^http:\/\/localhost:\d+\/$/);
  });

  test("deep link to issue detail works", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00001");
    await expect(
      page.getByText("i-seed00001", { exact: true })
    ).toBeVisible();
    await expect(
      page.getByText(/Platform v2\.0 Migration/)
    ).toBeVisible();
  });

  test("deep link to patch detail works", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/patches/p-seed00001");
    await expect(
      page.getByText("p-seed00001", { exact: true })
    ).toBeVisible();
    await expect(
      page.getByText("Add OAuth2 provider integration")
    ).toBeVisible();
  });

  test("browser back button works", async ({ authenticatedPage: page }) => {
    // Navigate to issues
    await page.goto("/issues");
    await expect(page).toHaveURL(/\/issues/);

    // Navigate to an issue detail
    await page.goto("/issues/i-seed00001");
    await expect(page).toHaveURL(/\/issues\/i-seed00001/);

    // Go back
    await page.goBack();
    await expect(page).toHaveURL(/\/issues$/);
  });
});
