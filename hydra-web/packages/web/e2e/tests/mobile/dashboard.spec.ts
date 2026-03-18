import { test, expect } from "../../fixtures/auth";

test.describe("Mobile Dashboard @mobile:dashboard", () => {
  test("dashboard renders and shows issues @mobile:dashboard", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=inbox");

    // Dashboard should show issue sections
    await expect(page.getByText(/Complete \(\d+\)/)).toBeVisible();

    // Issues should be visible
    await expect(
      page.getByText("Update deployment documentation")
    ).toBeVisible();
  });

  test("can click through to an issue from dashboard @mobile:dashboard", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=inbox");

    // Click on an issue to navigate to detail
    await page.getByText("Update deployment documentation").click();
    await expect(page).toHaveURL(/\/issues\/i-seed00010/);
  });
});
