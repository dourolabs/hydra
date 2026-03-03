import { test, expect } from "../fixtures/auth";

test.describe("Issue Detail", () => {
  test("displays issue description and metadata", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00002");
    // i-seed00002: "Migrate authentication to OAuth2"
    await expect(
      page.getByText("i-seed00002", { exact: true })
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Migrate authentication to OAuth2" })
    ).toBeVisible();
  });

  test("shows progress notes when present", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00002");
    // i-seed00002 has progress: "Provider selected (Keycloak)..."
    await expect(page.getByText(/Provider selected/)).toBeVisible();
  });

  test("shows breadcrumbs with link back to issues", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00002");
    const breadcrumb = page.locator('nav[aria-label="Breadcrumb"]');
    await expect(breadcrumb).toBeVisible();
    await expect(breadcrumb.getByText("Issues")).toBeVisible();
  });

  test("displays tabbed sections", async ({ authenticatedPage: page }) => {
    await page.goto("/issues/i-seed00001");
    // IssueDetail has tabs: Children, Jobs, Patches, Todo, Activity, Settings
    await expect(page.getByRole("tab", { name: "Children" })).toBeVisible();
    await expect(page.getByRole("tab", { name: "Jobs" })).toBeVisible();
    await expect(page.getByRole("tab", { name: "Patches" })).toBeVisible();
    await expect(page.getByRole("tab", { name: "Todo" })).toBeVisible();
  });

  test("shows todo list items when Todo tab is clicked", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00001");
    // Click the "Todo" tab
    await page.getByRole("tab", { name: "Todo" }).click();
    // i-seed00001 has todo items like "Finalize OAuth2 provider selection"
    await expect(
      page.getByText("Finalize OAuth2 provider selection")
    ).toBeVisible();
  });

  test("shows 404 for non-existent issue", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-nonexistent");
    await expect(page.getByText(/not found/i)).toBeVisible();
  });

  test("can update issue status via modal", async ({
    authenticatedPage: page,
  }) => {
    // Use i-seed00005 (closed) which is not referenced by badge tests
    await page.goto("/issues/i-seed00005");

    // Click the Update Status button
    const updateBtn = page.getByRole("button", { name: "Update Status" });
    await expect(updateBtn).toBeVisible();
    await updateBtn.click();

    // Modal should be open
    const modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();

    // Change status to "open"
    const statusSelect = modal.locator("select");
    await statusSelect.selectOption("open");

    // Click Save
    await modal.getByRole("button", { name: "Save" }).click();

    // Modal should close
    await expect(modal).not.toBeVisible();

    // The issue detail page should still be showing
    await expect(page.getByText("i-seed00005", { exact: true })).toBeVisible();
  });
});
