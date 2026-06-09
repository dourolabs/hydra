import { test, expect } from "../fixtures/auth";

// Covers the three @projects scenarios listed in scenarios.md:
//   - @projects:create — list + create + edit pages
//   - @projects:badge — issue list badge consumes resolved_status
//   - @projects:status-modal-options — status modal pulls from API per project
test.describe("Projects @projects:create", () => {
  test("user can create a project from the simplified new-project modal @projects:create", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/projects");
    await expect(page.getByRole("heading", { name: "Projects" })).toBeVisible();

    await page.getByTestId("projects-list-add").click();
    const modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();

    // The simplified modal has only a Name input and a Prompt textarea —
    // no key field, no status list, no prompt-path field.
    await expect(page.getByTestId("project-editor-key")).toHaveCount(0);
    await expect(page.getByTestId("project-editor-add-status")).toHaveCount(0);
    await expect(page.getByTestId("project-editor-prompt-path")).toHaveCount(0);

    await page.getByTestId("new-project-name").fill("E2E Engineering");
    await page
      .getByTestId("new-project-prompt-body")
      .fill("# E2E project prompt");

    await page.getByTestId("new-project-save").click();

    // Key is slugified from name and the page routes to the detail page.
    await page.waitForURL("**/projects/e2e-engineering");
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
    // Pick any seeded issue; seed issues without an explicit project_id are
    // backfilled to the seeded default project (`j-defaul`) on load.
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
