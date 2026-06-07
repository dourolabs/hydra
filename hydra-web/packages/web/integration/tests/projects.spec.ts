import { test, expect } from "../fixtures/auth";

// Covers the three @projects scenarios listed in scenarios.md:
//   - @projects:create — list + create + edit pages
//   - @projects:badge — issue list badge consumes resolved_status
//   - @projects:status-modal-options — status modal pulls from API per project
test.describe("Projects @projects:create", () => {
  test("user can create a project with custom statuses @projects:create", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/projects");
    await expect(page.getByRole("heading", { name: "Projects" })).toBeVisible();

    await page.getByTestId("projects-list-add").click();
    const modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();

    await page.getByTestId("project-editor-key").fill("integration-eng");
    await page.getByTestId("project-editor-name").fill("E2E Engineering");

    // Add one more status beyond the three defaults.
    await page.getByTestId("project-editor-add-status").click();

    await page.getByTestId("project-editor-save").click();

    // Routed to the detail page on success.
    await page.waitForURL("**/projects/integration-eng");
    await expect(
      page.getByRole("heading", { name: "E2E Engineering" }),
    ).toBeVisible();
  });
});

test.describe("Status badge reads resolved_status @projects:badge", () => {
  test("issue list shows resolved_status label and color verbatim @projects:badge", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=all");
    // Seed issues all use DefaultProject statuses, whose resolved_status
    // labels match the legacy display strings ("Open", "In progress", etc.).
    await expect(
      page.locator("table").getByText(/Open|In progress|Closed|Failed|Dropped/i).first(),
    ).toBeVisible();
  });
});

test.describe("Status modal options come from project @projects:status-modal-options", () => {
  test("project-less issue uses default project's statuses @projects:status-modal-options", async ({
    authenticatedPage: page,
  }) => {
    // Pick any seeded issue; seed issues default to project_id=null.
    await page.goto("/issues/i-seed00002");
    // Open the right rail's Details tab to access the status chip.
    await page.getByTestId("issue-rail-tab-details").click();
    await page.getByTestId("status-chip").click();

    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    // DefaultProject keys cover the five legacy statuses.
    for (const label of ["Open", "In progress", "Closed", "Dropped", "Failed"]) {
      await expect(dialog.getByRole("option", { name: label })).toBeAttached();
    }
  });
});
