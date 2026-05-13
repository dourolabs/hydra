import { test, expect } from "../fixtures/auth";

test.describe("Navigation @nav:sidebar @nav:deep-link @nav:back-button", () => {
  test("sidebar links navigate to correct pages @nav:sidebar", async ({
    authenticatedPage: page,
  }) => {
    // Navigate to Documents via the Documents section "More" link.
    await page.getByTestId("sidebar-section-documents-more").click();
    await expect(page).toHaveURL(/\/documents/);

    // Navigate to Settings via the Agents entry (Agents is a placeholder
    // pointing at /settings until phase 3 splits it out).
    await page.getByTestId("sidebar-agents").click();
    await expect(page).toHaveURL(/\/settings/);

    // Navigate to the dashboard via the Issues > All issues link.
    await page.getByTestId("sidebar-issues-all").click();
    await expect(page).toHaveURL(
      /^http:\/\/localhost:\d+\/\?selected=all$/,
    );
  });

  test("Issues section items deep-link to the dashboard @nav:sidebar", async ({
    authenticatedPage: page,
  }) => {
    // Assigned to you → dashboard with Assigned filter selected.
    await page.getByTestId("sidebar-issues-assigned").click();
    await expect(page).toHaveURL(
      /^http:\/\/localhost:\d+\/\?selected=assigned$/,
    );

    // All issues → dashboard with All filter selected.
    await page.getByTestId("sidebar-issues-all").click();
    await expect(page).toHaveURL(
      /^http:\/\/localhost:\d+\/\?selected=all$/,
    );

    // Clicking a recent-label row deep-links to ?selected=all&label=<id>.
    const labelRow = page.locator('[data-testid^="sidebar-issues-label-"]').first();
    await expect(labelRow).toBeVisible();
    const labelTestId = await labelRow.getAttribute("data-testid");
    const labelId = labelTestId!.replace("sidebar-issues-label-", "");
    await labelRow.click();
    await expect(page).toHaveURL(
      new RegExp(
        `^http://localhost:\\d+/\\?selected=all&label=${labelId}$`,
      ),
    );
  });

  test("deep link to issue detail works @nav:deep-link", async ({
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

  test("deep link to patch detail works @nav:deep-link", async ({
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

  test("browser back button works @nav:back-button", async ({ authenticatedPage: page }) => {
    // Navigate to dashboard
    await page.goto("/");
    await expect(page).toHaveURL(/\/\?selected=your-issues$/);

    // Navigate to an issue detail
    await page.goto("/issues/i-seed00001");
    await expect(page).toHaveURL(/\/issues\/i-seed00001/);

    // Go back
    await page.goBack();
    await expect(page).toHaveURL(/\/\?selected=your-issues$/);
  });
});
