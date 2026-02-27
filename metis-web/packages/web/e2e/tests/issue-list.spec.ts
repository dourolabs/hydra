import { test, expect } from "../fixtures/auth";

test.describe("Issue List", () => {
  test("renders seeded issues", async ({ authenticatedPage: page }) => {
    await page.goto("/issues");
    // Seed data contains issues like "Platform v2.0 Migration" and "Migrate authentication to OAuth2"
    await expect(
      page.getByText(/Platform v2\.0 Migration/)
    ).toBeVisible();
    await expect(
      page.getByText(/Migrate authentication to OAuth2/)
    ).toBeVisible();
  });

  test("displays status badges", async ({ authenticatedPage: page }) => {
    await page.goto("/issues");
    // Wait for issue tree to load
    await expect(
      page.getByText(/Platform v2\.0 Migration/)
    ).toBeVisible();

    // Verify badges for specific seeded issues across all status types
    // i-seed00001 = open, i-seed00002 = in-progress, i-seed00004 = closed,
    // i-seed00006 = failed, i-seed00010 = dropped
    const cases = [
      { desc: "Platform v2.0 Migration", status: "open" },
      { desc: "Migrate authentication to OAuth2", status: "in-progress" },
      { desc: "Add OAuth2 scopes and permissions support", status: "closed" },
      { desc: "Implement API rate limiting", status: "failed" },
      { desc: "Update deployment documentation", status: "dropped" },
    ];

    for (const { desc, status } of cases) {
      const row = page.getByRole("treeitem").filter({ hasText: desc });
      await expect(row.getByText(status, { exact: true })).toBeVisible();
    }
  });

  test("shows issue descriptions as snippets", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues");
    // Seed issue i-seed00001 has description about "Platform v2.0 Migration"
    await expect(page.getByText(/Platform v2\.0 Migration/)).toBeVisible();
  });

  test("issue rows are clickable and navigate to detail", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues");
    await expect(
      page.getByText(/Platform v2\.0 Migration/)
    ).toBeVisible();
    await page.getByText(/Platform v2\.0 Migration/).click();
    await expect(page).toHaveURL(/\/issues\/i-seed00001/);
  });
});
