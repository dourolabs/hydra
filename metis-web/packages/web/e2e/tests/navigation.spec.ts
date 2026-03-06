import { test, expect } from "../fixtures/auth";

test.describe("Navigation", () => {
  test("sidebar links navigate to correct pages", async ({
    authenticatedPage: page,
  }) => {
    // Navigate to Documents via sidebar
    await page.locator('nav a[href="/documents"]').click();
    await expect(page).toHaveURL(/\/documents/);

    // Navigate to Settings via sidebar
    await page.locator('nav a[href="/settings"]').click();
    await expect(page).toHaveURL(/\/settings/);

    // Navigate to Dashboard via sidebar
    await page.getByTestId('nav-dashboard').click();
    await expect(page).toHaveURL(/^http:\/\/localhost:\d+\/\?selected=inbox$/);
  });

  test("deep link to issue detail works", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00001");
    await expect(
      page.locator('nav[aria-label="Breadcrumb"]').getByText("i-seed00001")
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Platform v2.0 Migration" })
    ).toBeVisible();
  });

  test("deep link to patch detail works", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/patches/p-seed00001");
    await expect(
      page.getByRole("heading", { name: "Add OAuth2 provider integration" })
    ).toBeVisible();
    // patch_id is now in the Metadata tab
    await page.getByRole("tab", { name: "Metadata" }).click();
    await expect(
      page.getByText("p-seed00001", { exact: true })
    ).toBeVisible();
  });

  test("browser back button works", async ({ authenticatedPage: page }) => {
    // Navigate to dashboard
    await page.goto("/");
    await expect(page).toHaveURL(/\/\?selected=inbox$/);

    // Navigate to an issue detail
    await page.goto("/issues/i-seed00001");
    await expect(page).toHaveURL(/\/issues\/i-seed00001/);

    // Go back
    await page.goBack();
    await expect(page).toHaveURL(/\/\?selected=inbox$/);
  });
});
