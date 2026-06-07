import { test, expect } from "../fixtures/auth";

test.describe("Dashboard Search @dashboard:search", () => {
  test("search input retains focus and page does not flash while results load @dashboard:search", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=your-issues");

    // The free-text search box sits next to the FilterBar on the Issues list
    // and continues to drive a server-side `?q=` query.
    const searchInput = page.getByTestId("issues-search");
    await expect(searchInput).toBeVisible();

    // Verify initial items are visible before searching
    await expect(page.getByText("Fix login page 500 error on expired sessions")).toBeVisible();

    // Type a search query — the input should retain focus throughout
    await searchInput.click();
    await searchInput.fill("deployment");

    // The input should still be focused after typing
    await expect(searchInput).toBeFocused();

    // Wait for debounced search results to arrive (300ms debounce + network)
    await expect(page.getByText("Update deployment documentation")).toBeVisible({
      timeout: 5000,
    });

    // The search input should still have focus after results load
    await expect(searchInput).toBeFocused();

    // The input value should still be the search query (not reset)
    await expect(searchInput).toHaveValue("deployment");

    // After the debounce, the URL should persist the search term so the
    // result is shareable.
    await expect(page).toHaveURL(/[?&]q=deployment\b/);
  });

  test("user can open the add-filter menu and pick a Status filter @dashboard:search", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=your-issues");

    // FilterBar lives alongside the free-text search box. Smoke-test that the
    // add-filter affordance is reachable and the resulting popover contains
    // the expected status options.
    const addFilter = page.getByTestId("filter-bar-add");
    await expect(addFilter).toBeVisible();

    await addFilter.click();
    await expect(page.getByTestId("add-filter-menu")).toBeVisible();
    await expect(page.getByTestId("add-filter-status")).toBeVisible();

    await page.getByTestId("add-filter-status").click();

    // The chip should now exist and the value picker should be open.
    await expect(page.getByTestId("filter-chip-status")).toBeVisible();
    await expect(page.getByTestId("value-picker-status")).toBeVisible();

    // Picking a value should narrow server-side and persist to the URL.
    await page.getByTestId("value-option-open").click();
    await expect(page).toHaveURL(/[?&]status=open\b/);
  });
});
