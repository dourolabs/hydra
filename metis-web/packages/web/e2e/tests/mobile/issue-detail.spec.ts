import { test, expect } from "../../fixtures/auth";

test.describe("Mobile Issue Detail @mobile:issue-detail", () => {
  test("displays issue heading and breadcrumbs @mobile:issue-detail", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00002");
    await expect(
      page.locator('nav[aria-label="Breadcrumb"]').getByText("i-seed00002")
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Migrate authentication to OAuth2" })
    ).toBeVisible();
  });

  test("tabs are accessible and clickable @mobile:issue-detail", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00001");

    // All tabs should be visible
    await expect(page.getByRole("tab", { name: "Related Issues" })).toBeVisible();
    await expect(page.getByRole("tab", { name: "Patches" })).toBeVisible();
    await expect(page.getByRole("tab", { name: "Metadata" })).toBeVisible();

    // Can click a tab
    await page.getByRole("tab", { name: "Metadata" }).click();
    await expect(page.getByText("i-seed00001")).toBeVisible();
  });

  test("status chip is accessible @mobile:issue-detail", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00005");

    const statusChip = page.getByTestId("status-chip");
    await expect(statusChip).toBeVisible();

    // Can open the status update modal
    await statusChip.click();
    const modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();
  });
});
