import { test, expect } from "../fixtures/auth";

test.describe("Navigation @nav:sidebar @nav:deep-link @nav:back-button @nav:sidebar-toggle", () => {
  test("sidebar links navigate to correct pages @nav:sidebar", async ({
    authenticatedPage: page,
  }) => {
    // Navigate to Documents via the Documents workspace entry.
    await page.getByTestId("sidebar-documents").click();
    await expect(page).toHaveURL(/\/documents/);

    // Navigate to the Agents page via the Agents sidebar entry.
    await page.getByTestId("sidebar-agents").click();
    await expect(page).toHaveURL(/\/agents/);

    // Navigate to the Sessions list via the site-header active-sessions slot.
    // The Sessions FilterBar auto-seeds a `?creator=users/<me>` chip on first
    // paint (Mine-as-default), so the URL may include that param.
    await page.getByTestId("site-header-sessions").click();
    await expect(page).toHaveURL(/\/sessions(\?|$)/);

    // Workspace > Issues link → all-issues landing page.
    await page.getByTestId("sidebar-issues-all").click();
    await expect(page).toHaveURL(/^http:\/\/localhost:\d+\/$/);

    // The Views > My issues link scopes the dashboard to the current user.
    await page.getByTestId("sidebar-issues-your-issues").click();
    await expect(page).toHaveURL(
      /^http:\/\/localhost:\d+\/\?creator=[^&]+$/,
    );
  });

  test("Issues section items deep-link to the dashboard @nav:sidebar", async ({
    authenticatedPage: page,
  }) => {
    // Workspace > Issues → bare dashboard with no filters.
    await page.getByTestId("sidebar-issues-all").click();
    await expect(page).toHaveURL(/^http:\/\/localhost:\d+\/$/);

    // Assigned to you → dashboard with Assigned filter selected. The URL
    // carries the Principal path form (`users/<x>` / `agents/<x>`) — bare
    // names are rejected by the server's typed deserializer (Phase 4b).
    await page.getByTestId("sidebar-issues-assigned").click();
    await expect(page).toHaveURL(
      /^http:\/\/localhost:\d+\/\?assignee=(users|agents)%2F[^&]+$/,
    );
    // The list should actually render (at least one issue row, the dropped
    // "Update deployment documentation" item assigned to dev-user). If the
    // request 400s because of a malformed assignee param, the table is empty.
    await expect(
      page.getByText("Update deployment documentation"),
    ).toBeVisible();

    // My issues → dashboard scoped to the current user as creator.
    await page.getByTestId("sidebar-issues-your-issues").click();
    await expect(page).toHaveURL(
      /^http:\/\/localhost:\d+\/\?creator=[^&]+$/,
    );
  });

  test("deep link to issue detail works @nav:deep-link", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00001");
    // Breadcrumb's trailing crumb is the issue ID; the title is the page heading.
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
    // patch_id is rendered in the title block; no Metadata tab anymore.
    // Scope to <main> since the breadcrumb also shows the patch ID.
    await expect(
      page.getByRole("main").getByText("p-seed00001", { exact: true })
    ).toBeVisible();
  });

  test("sidebar hide/restore cycle keeps page functional @nav:sidebar-toggle", async ({
    authenticatedPage: page,
  }) => {
    const sidebar = page.getByTestId("sidebar");
    const chromeToggle = page.getByTestId("site-header-toggle-sidebar");
    const brand = page.getByTestId("hydra-brand");

    // Initially the sidebar is visible; the chrome hamburger toggles it and
    // the Hydra wordmark sits next to it. The chrome is pinned regardless of
    // sidebar state.
    await expect(sidebar).toBeVisible();
    await expect(chromeToggle).toBeVisible();
    await expect(chromeToggle).toHaveAttribute("aria-label", "Hide sidebar");
    await expect(brand).toBeVisible();
    const initialBox = await sidebar.boundingBox();
    expect(initialBox?.width ?? 0).toBeGreaterThan(100);
    const initialBrandBox = await brand.boundingBox();

    // Clicking the chrome hamburger hides the sidebar; the same button now
    // acts as the "Show sidebar" restore control and stays in the same spot.
    await chromeToggle.click();
    await expect
      .poll(async () => (await sidebar.boundingBox())?.width ?? -1)
      .toBeLessThan(5);
    await expect(chromeToggle).toBeVisible();
    await expect(chromeToggle).toHaveAttribute("aria-label", "Show sidebar");
    const hiddenBrandBox = await brand.boundingBox();
    expect(hiddenBrandBox?.x).toBe(initialBrandBox?.x);

    // localStorage records the hidden state for next reload.
    expect(
      await page.evaluate(() => localStorage.getItem("hydra-sidebar-hidden")),
    ).toBe("1");

    // After reload, the sidebar stays hidden and the chrome toggle is still
    // present with the same aria-label.
    await page.reload();
    await expect
      .poll(async () => (await sidebar.boundingBox())?.width ?? -1)
      .toBeLessThan(5);
    await expect(chromeToggle).toBeVisible();
    await expect(chromeToggle).toHaveAttribute("aria-label", "Show sidebar");

    // Restoring brings the sidebar back. The chrome toggle stays put and the
    // main content is still functional, shown by navigating via the
    // Documents link in the sidebar.
    await chromeToggle.click();
    await expect
      .poll(async () => (await sidebar.boundingBox())?.width ?? 0)
      .toBeGreaterThan(100);
    await expect(chromeToggle).toBeVisible();
    await expect(chromeToggle).toHaveAttribute("aria-label", "Hide sidebar");
    expect(
      await page.evaluate(() => localStorage.getItem("hydra-sidebar-hidden")),
    ).toBe("0");

    await page.getByTestId("sidebar-documents").click();
    await expect(page).toHaveURL(/\/documents/);
  });

  test("browser back button works @nav:back-button", async ({ authenticatedPage: page }) => {
    // Navigate to dashboard at root (URL is not auto-rewritten with ?selected=).
    await page.goto("/");
    await expect(page).toHaveURL(/\/$/);

    // Navigate to an issue detail
    await page.goto("/issues/i-seed00001");
    await expect(page).toHaveURL(/\/issues\/i-seed00001/);

    // Go back
    await page.goBack();
    await expect(page).toHaveURL(/\/$/);
  });
});
