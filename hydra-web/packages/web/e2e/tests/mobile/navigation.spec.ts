import { test, expect } from "../../fixtures/auth";

test.describe("Mobile Navigation @mobile:nav", () => {
  test("sidebar links navigate to correct pages @mobile:nav", async ({
    authenticatedPage: page,
  }) => {
    // Navigate to Documents via sidebar
    await page.locator('nav a[href="/documents"]').click();
    await expect(page).toHaveURL(/\/documents/);

    // Navigate to Settings via sidebar
    await page.locator('nav a[href="/settings"]').click();
    await expect(page).toHaveURL(/\/settings/);

    // Navigate to Dashboard via sidebar
    await page.getByTestId("nav-dashboard").click();
    await expect(page).toHaveURL(/^http:\/\/localhost:\d+\/\?selected=inbox$/);
  });

  test("deep link to issue detail renders correctly @mobile:nav", async ({
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

  test("deep link to patch detail renders correctly @mobile:nav", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/patches/p-seed00001");
    await expect(
      page.getByRole("heading", { name: "Add OAuth2 provider integration" })
    ).toBeVisible();
  });
});
