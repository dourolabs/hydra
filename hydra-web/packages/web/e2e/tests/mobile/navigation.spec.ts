import { test, expect } from "../../fixtures/auth";

test.describe("Mobile Navigation @mobile:nav", () => {
  test("sidebar links navigate to correct pages @mobile:nav", async ({
    authenticatedPage: page,
  }) => {
    // Navigate to Documents via the Documents section "More" link.
    await page.getByTestId("sidebar-section-documents-more").click();
    await expect(page).toHaveURL(/\/documents/);

    // Navigate to the Agents page via the Agents sidebar entry.
    await page.getByTestId("sidebar-agents").click();
    await expect(page).toHaveURL(/\/agents/);

    // Navigate to the dashboard via the Issues > All issues link.
    await page.getByTestId("sidebar-issues-all").click();
    await expect(page).toHaveURL(
      /^http:\/\/localhost:\d+\/\?selected=all$/,
    );
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
