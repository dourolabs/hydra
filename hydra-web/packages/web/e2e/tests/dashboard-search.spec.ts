import { test, expect } from "../fixtures/auth";

test.describe("Dashboard Search @dashboard:search", () => {
  test("search input retains focus and page does not flash while results load @dashboard:search", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=inbox");

    // Wait for the dashboard to fully load
    const searchInput = page.getByPlaceholder("Search issues...");
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
  });
});
