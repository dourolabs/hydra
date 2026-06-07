import { test, expect } from "../fixtures/auth";

test.describe("Global search modal @global-search", () => {
  test("opens with Cmd-K, finds results, navigates on click, closes on Escape", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/");
    await expect(page.getByTestId("sidebar")).toBeVisible();

    // Modal is not visible initially.
    await expect(page.getByTestId("global-search-modal")).toHaveCount(0);

    // Ensure focus is in the document so the window keydown listener sees it.
    await page.locator("body").click({ position: { x: 5, y: 5 } });

    // Cmd-K / Ctrl-K opens the modal. ControlOrMeta picks the platform-native modifier.
    await page.keyboard.press("ControlOrMeta+k");
    await expect(page.getByTestId("global-search-modal")).toBeVisible();
    const input = page.getByTestId("global-search-input");
    await expect(input).toBeFocused();

    // Same shortcut again toggles closed.
    await page.keyboard.press("ControlOrMeta+k");
    await expect(page.getByTestId("global-search-modal")).toHaveCount(0);

    // Re-open and type a query that should hit seeded fixtures.
    await page.keyboard.press("ControlOrMeta+k");
    await expect(page.getByTestId("global-search-modal")).toBeVisible();
    await input.fill("OAuth");

    // Expect at least one Issues row (seed contains multiple OAuth-related issues).
    await expect(page.getByTestId("global-search-group-issue")).toBeVisible({
      timeout: 5000,
    });
    const firstIssueRow = page
      .locator('[data-testid^="global-search-row-issue-"]')
      .first();
    await expect(firstIssueRow).toBeVisible();

    // Capture the href the row would navigate to via its data-testid.
    const testId = await firstIssueRow.getAttribute("data-testid");
    expect(testId).toMatch(/^global-search-row-issue-/);
    const issueId = testId!.replace("global-search-row-issue-", "");

    // Click navigates to the issue detail and closes the modal.
    await firstIssueRow.click();
    await expect(page).toHaveURL(new RegExp(`/issues/${issueId}$`));
    await expect(page.getByTestId("global-search-modal")).toHaveCount(0);

    // Reopen, then press Escape to close.
    await page.keyboard.press("ControlOrMeta+k");
    await expect(page.getByTestId("global-search-modal")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("global-search-modal")).toHaveCount(0);
  });

  test("clicking the site-header magnifying-glass opens the modal", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/");
    await expect(page.getByTestId("global-search-modal")).toHaveCount(0);
    await page.getByTestId("site-header-search").click();
    await expect(page.getByTestId("global-search-modal")).toBeVisible();
    await expect(page.getByTestId("global-search-input")).toBeFocused();
  });
});
