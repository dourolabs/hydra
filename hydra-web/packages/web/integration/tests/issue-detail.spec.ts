import { test, expect } from "../fixtures/auth";

test.describe("Issue Detail @issues:view-detail @issues:update-status @issues:navigate-tabs @issues:blocked-tag @projects:details-rail-project-block @errors:404", () => {
  test("displays issue description and metadata @issues:view-detail", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00002");
    // i-seed00002: "Migrate authentication to OAuth2"
    // Breadcrumb shows the issue ID as the trailing crumb; the title is the page heading.
    await expect(
      page.locator('nav[aria-label="Breadcrumb"]').getByText("i-seed00002")
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Migrate authentication to OAuth2" })
    ).toBeVisible();
  });

  test("shows progress notes when present @issues:view-detail", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00002");
    // i-seed00002 has progress: "Provider selected (Keycloak)..."
    await expect(page.getByText(/Provider selected/)).toBeVisible();
  });

  test("shows breadcrumbs with link back to issues @issues:view-detail", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00002");
    const breadcrumb = page.locator('nav[aria-label="Breadcrumb"]');
    await expect(breadcrumb).toBeVisible();
    await expect(breadcrumb.getByText("Workspace")).toBeVisible();
    await expect(breadcrumb.getByText("Issues")).toBeVisible();
  });

  test("displays tabbed sections @issues:navigate-tabs", async ({ authenticatedPage: page }) => {
    await page.goto("/issues/i-seed00001");
    // The right-rail panel exposes three tabs: Related | Activity | Details.
    await expect(page.getByTestId("issue-rail-tab-related")).toBeVisible();
    await expect(page.getByTestId("issue-rail-tab-activity")).toBeVisible();
    await expect(page.getByTestId("issue-rail-tab-details")).toBeVisible();

    // Related is the default active tab; at least one section heading should
    // render (Parents/Children/Patches/Documents — empty states still show heads).
    await expect(page.getByRole("heading", { name: /Parents|Children|Patches|Documents/ }).first())
      .toBeVisible();
  });

  test("Details tab renders a Project row with a ProjectChip @projects:details-rail-project-block", async ({
    authenticatedPage: page,
  }) => {
    // i-seed00012 is scoped to project j-engv2 (Engineering v2).
    await page.goto("/issues/i-seed00012");
    await page.getByTestId("issue-rail-tab-details").click();
    await expect(page.getByText("Project", { exact: true })).toBeVisible();
    await expect(page.getByText("Engineering v2")).toBeVisible();
  });

  test("Details tab falls back to the default project when issue has no project_id @projects:details-rail-project-block", async ({
    authenticatedPage: page,
  }) => {
    // i-seed00001 has no project_id; the row should render the default
    // project chip (project_id=j-defaul, key=default, name=Default — seeded
    // in mock-server fixtures to mirror the real backend's
    // `seed_default_project` migration row).
    await page.goto("/issues/i-seed00001");
    await page.getByTestId("issue-rail-tab-details").click();
    await expect(page.getByText("Project", { exact: true })).toBeVisible();
    await expect(page.getByText("Default", { exact: true })).toBeVisible();
  });

  test("Details tab shows BLOCKED tag when blocked-on dep is open @issues:blocked-tag", async ({
    authenticatedPage: page,
  }) => {
    // i-seed00012 has a blocked-on dep on i-seed00006 (in-progress).
    await page.goto("/issues/i-seed00012");
    await page.getByTestId("issue-rail-tab-details").click();
    await expect(page.getByTestId("blocked-tag")).toBeVisible();
    await expect(page.getByTestId("blocked-tag")).toHaveText("BLOCKED");
  });

  test("Details tab omits BLOCKED tag for issues with no open blockers @issues:blocked-tag", async ({
    authenticatedPage: page,
  }) => {
    // i-seed00001 has no blocked-on deps.
    await page.goto("/issues/i-seed00001");
    await page.getByTestId("issue-rail-tab-details").click();
    await expect(page.getByTestId("blocked-tag")).toHaveCount(0);
  });

  test("shows 404 for non-existent issue @errors:404", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-nonexistent");
    await expect(page.getByText(/not found/i)).toBeVisible();
  });

  test("can update issue status via modal @issues:update-status", async ({
    authenticatedPage: page,
  }) => {
    // Use i-seed00005 (closed) which is not referenced by badge tests
    await page.goto("/issues/i-seed00005");

    // Status chip lives in the Details tab — activate it first.
    await page.getByTestId("issue-rail-tab-details").click();

    // Click the status chip to open the update modal
    const statusChip = page.getByTestId("status-chip");
    await expect(statusChip).toBeVisible();
    await statusChip.click();

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

    // The issue detail page should still be showing — breadcrumb's trailing
    // crumb is the issue ID.
    await expect(
      page.locator('nav[aria-label="Breadcrumb"]').getByText("i-seed00005")
    ).toBeVisible();
  });

  test("can reassign via the inline assignee dropdown in the meta row", async ({
    authenticatedPage: page,
  }) => {
    // i-seed00002 starts assigned to agent "swe".
    await page.goto("/issues/i-seed00002");

    const picker = page.getByTestId("issue-assignee-picker");
    await expect(picker).toBeVisible();

    // The bare dropdown is rendered without a visible "ASSIGNEE" caption.
    await expect(picker.getByText("ASSIGNEE", { exact: true })).toHaveCount(0);

    const trigger = picker.getByRole("button", { name: "Assignee" });
    await expect(trigger).toContainText("swe");

    await trigger.click();
    // Pick a human user from the (portaled) popover. The row carries a stable
    // testid so we sidestep accessible-name collisions with related-rail rows.
    await page.getByTestId("issue-assignee-option-user-alice").click();

    // After reassignment the trigger reflects the new principal.
    await expect(trigger).toContainText("alice");
  });
});
