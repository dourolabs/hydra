import { test, expect } from "../fixtures/auth";

test.describe("Dashboard FilterBar @dashboard:search", () => {
  test("user can open the add-filter menu from the Issues list and pick a Status filter @dashboard:search", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=your-issues");

    // The new FilterBar replaced the old per-page filter Pickers. Smoke-test
    // that the add-filter affordance is reachable and the resulting popover
    // contains the expected status options.
    const addFilter = page.getByTestId("filter-bar-add");
    await expect(addFilter).toBeVisible();

    await addFilter.click();
    await expect(page.getByTestId("add-filter-menu")).toBeVisible();
    await expect(page.getByTestId("add-filter-status")).toBeVisible();

    await page.getByTestId("add-filter-status").click();

    // The chip should now exist and the value picker should be open.
    await expect(page.getByTestId("filter-chip-status")).toBeVisible();
    await expect(page.getByTestId("value-picker-status")).toBeVisible();
  });
});
