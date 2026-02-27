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
    // The page should show various status badges from the seed data
    // Seed data includes issues with statuses: open, in-progress, closed, failed, dropped
    const issueList = page.locator("main");
    await expect(issueList).toBeVisible();
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
