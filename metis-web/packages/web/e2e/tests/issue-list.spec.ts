import { test, expect } from "../fixtures/auth";

test.describe("Issue List", () => {
  test("renders seeded issues", async ({ authenticatedPage: page }) => {
    await page.goto("/issues");
    // Seed data contains issues like "Platform v2.0 Migration" and "Migrate authentication to OAuth2"
    await expect(
      page.getByText("i-seed00001", { exact: true })
    ).toBeVisible();
    await expect(
      page.getByText("i-seed00002", { exact: true })
    ).toBeVisible();
  });

  test("displays status badges", async ({ authenticatedPage: page }) => {
    await page.goto("/issues");
    // Wait for issue tree to load
    await expect(
      page.getByText("i-seed00001", { exact: true })
    ).toBeVisible();

    // Verify badges for specific seeded issues across all status types
    // i-seed00001 = open, i-seed00002 = in-progress, i-seed00004 = closed,
    // i-seed00006 = failed, i-seed00010 = dropped
    const cases = [
      { id: "i-seed00001", status: "open" },
      { id: "i-seed00002", status: "in-progress" },
      { id: "i-seed00004", status: "closed" },
      { id: "i-seed00006", status: "failed" },
      { id: "i-seed00010", status: "dropped" },
    ];

    for (const { id, status } of cases) {
      const row = page.getByRole("treeitem").filter({ hasText: id });
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
      page.getByText("i-seed00001", { exact: true })
    ).toBeVisible();
    await page.getByText("i-seed00001", { exact: true }).click();
    await expect(page).toHaveURL(/\/issues\/i-seed00001/);
  });
});
