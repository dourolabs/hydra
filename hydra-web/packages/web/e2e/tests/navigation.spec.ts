import { test, expect } from "../fixtures/auth";

test.describe("Navigation @nav:sidebar @nav:deep-link @nav:back-button @nav:sidebar-toggle", () => {
  test("sidebar links navigate to correct pages @nav:sidebar", async ({
    authenticatedPage: page,
  }) => {
    // Navigate to Documents via the Documents section "More" link.
    await page.getByTestId("sidebar-section-documents-more").click();
    await expect(page).toHaveURL(/\/documents/);

    // Navigate to the Agents page via the Agents sidebar entry.
    await page.getByTestId("sidebar-agents").click();
    await expect(page).toHaveURL(/\/agents/);

    // Navigate to the Sessions list via the site-header active-sessions slot.
    await page.getByTestId("site-header-sessions").click();
    await expect(page).toHaveURL(/\/sessions$/);

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

  test("sidebar hide/restore cycle keeps page functional @nav:sidebar-toggle", async ({
    authenticatedPage: page,
  }) => {
    const sidebar = page.getByTestId("sidebar");
    const sidebarHideButton = page.getByTestId("sidebar-hide");
    const headerToggleButton = page.getByTestId("site-header-toggle-sidebar");

    // Initially the sidebar is visible and has non-zero width. On desktop the
    // hide control lives inside the sidebar; the header hamburger is absent.
    await expect(sidebar).toBeVisible();
    await expect(sidebarHideButton).toBeVisible();
    await expect(headerToggleButton).toHaveCount(0);
    const initialBox = await sidebar.boundingBox();
    expect(initialBox?.width ?? 0).toBeGreaterThan(100);

    // Clicking the sidebar hide button collapses the sidebar; the header
    // hamburger now mounts and exposes the "Show sidebar" affordance.
    await sidebarHideButton.click();
    await expect
      .poll(async () => (await sidebar.boundingBox())?.width ?? -1)
      .toBeLessThan(5);
    await expect(headerToggleButton).toBeVisible();
    await expect(headerToggleButton).toHaveAttribute(
      "aria-label",
      "Show sidebar",
    );

    // localStorage records the hidden state for next reload.
    expect(
      await page.evaluate(() => localStorage.getItem("hydra-sidebar-hidden")),
    ).toBe("1");

    // After reload, the sidebar stays hidden and the header toggle still says
    // "Show".
    await page.reload();
    await expect
      .poll(async () => (await sidebar.boundingBox())?.width ?? -1)
      .toBeLessThan(5);
    await expect(headerToggleButton).toHaveAttribute(
      "aria-label",
      "Show sidebar",
    );

    // Restoring brings the sidebar back; the header hamburger unmounts and
    // main content is still functional, as shown by navigation via the
    // Documents "More" link.
    await headerToggleButton.click();
    await expect
      .poll(async () => (await sidebar.boundingBox())?.width ?? 0)
      .toBeGreaterThan(100);
    await expect(headerToggleButton).toHaveCount(0);
    await expect(sidebarHideButton).toBeVisible();
    expect(
      await page.evaluate(() => localStorage.getItem("hydra-sidebar-hidden")),
    ).toBe("0");

    await page.getByTestId("sidebar-section-documents-more").click();
    await expect(page).toHaveURL(/\/documents/);
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
