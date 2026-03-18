import { test, expect } from "../fixtures/auth";

test.describe("Label display on dashboard item rows @labels:display", () => {
  test("shows label chips on issues with labels @labels:display", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=everything");
    await expect(page.getByText("Platform v2.0 Migration")).toBeVisible();

    // Seed data: i-seed00001 has "platform-v2", i-seed00002 has "platform-v2" + "auth",
    // i-seed00006 has "infra"
    const rows = page.locator("li[role=button]");
    await expect(rows.filter({ hasText: "platform-v2" }).first()).toBeVisible();
    await expect(rows.filter({ hasText: "auth" }).first()).toBeVisible();
    await expect(rows.filter({ hasText: "infra" }).first()).toBeVisible();
  });

  test("label chips appear within their respective issue rows @labels:display", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=everything");
    await expect(page.getByText("Platform v2.0 Migration")).toBeVisible();

    // Verify "infra" label chip appears in the i-seed00006 row
    const rateRow = page.locator("li[role=button]").filter({
      hasText: "Implement API rate limiting",
    });
    await expect(rateRow.getByText("infra")).toBeVisible();

    // Verify "platform-v2" label chip appears in the i-seed00001 row
    const migrationRow = page.locator("li[role=button]").filter({
      hasText: "Platform v2.0 Migration",
    });
    await expect(migrationRow.getByText("platform-v2")).toBeVisible();
  });
});

test.describe("Creating an issue with labels via LabelPicker @labels:create-with", () => {
  test("creates an issue with an existing and a new label @labels:create-with", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=everything");
    await expect(page.getByText("Platform v2.0 Migration")).toBeVisible();

    // Open the create issue modal
    await page.getByRole("button", { name: "+ Create Issue" }).click();
    const modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();

    // Fill in required fields
    await modal
      .getByPlaceholder("Short summary (optional)")
      .fill("Test issue with labels");
    await modal
      .getByPlaceholder("Describe the issue...")
      .fill("E2E test for label creation workflow");

    // Select an existing label by typing to filter
    const labelInput = modal.getByPlaceholder("Add labels...");
    await labelInput.click();
    await labelInput.fill("plat");
    const existingOption = modal.locator("li").filter({ hasText: "platform-v2" });
    await expect(existingOption).toBeVisible();
    await existingOption.click();

    // Create a new label by typing a name that doesn't exist
    // After selecting a label the placeholder is empty, so locate the input
    // relative to the "Labels" heading
    const pickerInput = modal
      .getByText("Labels", { exact: true })
      .locator("..")
      .locator("input");
    await pickerInput.fill("e2e-test-label");
    const createOption = modal.locator("li").filter({ hasText: /Create/ });
    await expect(createOption).toBeVisible();
    await createOption.click();

    // Verify both labels appear as chips in the modal
    await expect(modal.getByText("platform-v2")).toBeVisible();
    await expect(modal.getByText("e2e-test-label")).toBeVisible();

    // Close the dropdown by clicking the title field (away from the picker)
    await modal.getByPlaceholder("Short summary (optional)").click();

    // Submit the form
    await modal.getByRole("button", { name: "Create Issue" }).click();

    // Modal should close
    await expect(modal).not.toBeVisible();

    // Success toast should appear
    await expect(page.getByText(/Issue .+ created/)).toBeVisible();
  });
});

test.describe("Hidden labels are excluded from all user-facing label UI @labels:hidden", () => {
  // i-seed00008 ("Add dark mode support") has visible "platform-v2" and hidden "inbox" labels
  test("hidden labels do not appear in issue detail label display @labels:hidden", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00008");
    await expect(
      page.getByRole("heading", { name: "Add dark mode support" })
    ).toBeVisible();

    const labelSection = page.getByTestId("label-editor");
    await expect(labelSection.getByText("platform-v2")).toBeVisible();
    await expect(labelSection.getByText("inbox")).not.toBeVisible();
  });

  test("hidden labels do not appear as selected chips in edit mode @labels:hidden", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00008");
    await expect(
      page.getByRole("heading", { name: "Add dark mode support" })
    ).toBeVisible();

    // Enter edit mode
    await page.getByRole("button", { name: "Edit labels" }).click();

    // The editable area should show platform-v2 but NOT inbox
    const editorArea = page.getByText("Labels", { exact: true }).locator("..").locator("..");
    await expect(editorArea.getByText("platform-v2")).toBeVisible();
    await expect(editorArea.getByText("inbox")).not.toBeVisible();
  });

  test("saving labels in edit mode preserves hidden labels @labels:hidden", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00008");
    await expect(
      page.getByRole("heading", { name: "Add dark mode support" })
    ).toBeVisible();

    // Enter edit mode
    await page.getByRole("button", { name: "Edit labels" }).click();

    // Save without changes
    await page.getByRole("button", { name: "Save" }).click();

    // After saving, the visible label should still be displayed
    const labelSection = page.getByTestId("label-editor");
    await expect(labelSection.getByText("platform-v2")).toBeVisible();
    // Hidden "inbox" label should still not be visible (but preserved on the issue)
    await expect(labelSection.getByText("inbox")).not.toBeVisible();

    // Re-enter edit mode to confirm visible labels are still correct
    await page.getByRole("button", { name: "Edit labels" }).click();
    const editorArea = page.getByText("Labels", { exact: true }).locator("..").locator("..");
    await expect(editorArea.getByText("platform-v2")).toBeVisible();
    await expect(editorArea.getByText("inbox")).not.toBeVisible();
  });

  test("hidden labels do not appear in dashboard issue rows @labels:hidden", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=everything");

    // Use CSS class selector to target ItemRow elements specifically (not sidebar items)
    const darkModeRow = page.locator('li[class*="row"]').filter({
      hasText: "Add dark mode support",
    });
    await expect(darkModeRow).toBeVisible();
    // i-seed00008 has visible "platform-v2" and hidden "inbox" labels
    await expect(darkModeRow.getByText("platform-v2")).toBeVisible();
    await expect(darkModeRow.getByText("inbox")).not.toBeVisible();
  });

  test("hidden labels do not appear in label picker dropdown @labels:hidden", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=everything");
    await expect(page.getByText("Platform v2.0 Migration")).toBeVisible();

    // Open the create issue modal
    await page.getByRole("button", { name: "+ Create Issue" }).click();
    const modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();

    // Open the label picker and search for "inbox"
    const labelInput = modal.getByPlaceholder("Add labels...");
    await labelInput.click();
    await labelInput.fill("inbox");

    // Should show "Create" option but not an existing "inbox" label
    const inboxOption = modal.locator("li").filter({ hasText: /^inbox$/ });
    await expect(inboxOption).not.toBeVisible();
  });

  test("hidden labels do not appear in sidebar label filter section @labels:hidden", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=everything");
    await expect(page.getByText("Platform v2.0 Migration")).toBeVisible();

    // The sidebar Labels section should list visible labels but not "inbox"
    // Use exact match to distinguish from the "Inbox" navigation item
    const sidebar = page.locator('[class*="sidebar"]');
    await expect(sidebar.getByText("Labels")).toBeVisible();
    await expect(sidebar.getByText("platform-v2")).toBeVisible();
    // The label section header "Labels" is visible, but "inbox" as a label entry should not be
    await expect(sidebar.getByText("inbox", { exact: true })).not.toBeVisible();
  });
});

test.describe("Newly created label appears in sidebar @labels:sidebar-create", () => {
  test("label created during issue creation shows in sidebar @labels:sidebar-create", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=everything");
    await expect(page.getByText("Platform v2.0 Migration")).toBeVisible();

    // Open the create issue modal
    await page.getByRole("button", { name: "+ Create Issue" }).click();
    const modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();

    // Fill in required fields
    await modal
      .getByPlaceholder("Short summary (optional)")
      .fill("Sidebar label test issue");
    await modal
      .getByPlaceholder("Describe the issue...")
      .fill("Testing that new labels appear in sidebar");

    // Create a new label via the LabelPicker
    const labelInput = modal.getByPlaceholder("Add labels...");
    await labelInput.click();
    await labelInput.fill("sidebar-test-label");
    const createOption = modal.locator("li").filter({ hasText: /Create/ });
    await expect(createOption).toBeVisible();
    await createOption.click();

    // Verify label chip appears in modal
    await expect(modal.getByText("sidebar-test-label")).toBeVisible();

    // Close dropdown by clicking title field
    await modal.getByPlaceholder("Short summary (optional)").click();

    // Submit the form
    await modal.getByRole("button", { name: "Create Issue" }).click();
    await expect(modal).not.toBeVisible();
    await expect(page.getByText(/Issue .+ created/)).toBeVisible();

    // Reload to ensure the labels list is refreshed from the server
    await page.reload();
    await expect(page.getByText("Platform v2.0 Migration")).toBeVisible();

    // Verify the new label appears in the sidebar under Labels section
    // Use the stats pattern (e.g. "0/1") to target the sidebar label item,
    // not the LabelChip on the issue row
    await expect(
      page.getByRole("button", { name: /^sidebar-test-label \d+\/\d+$/ }),
    ).toBeVisible();
  });
});

test.describe("Filter by label in sidebar shows issue with badge @labels:filter", () => {
  test("clicking a label in sidebar filters dashboard and shows label chip on issue @labels:filter", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=everything");
    await expect(page.getByText("Platform v2.0 Migration")).toBeVisible();

    // The seed data has "infra" label on i-seed00006 ("Implement API rate limiting")
    // Click the "infra" label in the sidebar to filter
    // Use the accessible name which includes the label name and stats (e.g. "infra 0/1")
    const sidebarInfra = page.getByRole("button", { name: /^infra \d+\/\d+$/ });
    await expect(sidebarInfra).toBeVisible();
    await sidebarInfra.click();

    // URL should update to contain label filter
    await expect(page).toHaveURL(/selected=label(%3A|:)/);

    // The filtered list should show the issue with the "infra" label
    const issueRow = page.locator("li[role=button]").filter({
      hasText: "Implement API rate limiting",
    });
    await expect(issueRow).toBeVisible();

    // Verify the issue row has a LabelChip with "infra"
    await expect(issueRow.getByText("infra")).toBeVisible();
  });
});

test.describe("Editing labels on issue detail page via IssueLabelEditor @labels:edit", () => {
  test("displays existing labels on issue detail @labels:display @labels:edit", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00002");
    await expect(
      page.getByRole("heading", {
        name: "Migrate authentication to OAuth2",
      })
    ).toBeVisible();

    // i-seed00002 has labels "platform-v2" and "auth"
    // Scope to the label editor section to avoid matching labels inside ItemRow chips
    const labelSection = page.getByTestId("label-editor");
    await expect(labelSection.getByText("platform-v2")).toBeVisible();
    await expect(labelSection.getByText("auth", { exact: true })).toBeVisible();
  });

  test("can add and remove labels via the editor @labels:edit", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00002");
    await expect(
      page.getByRole("heading", {
        name: "Migrate authentication to OAuth2",
      })
    ).toBeVisible();

    // Enter edit mode
    await page.getByRole("button", { name: "Edit labels" }).click();

    // Verify existing labels are shown as editable chips
    // Use the label picker area to scope selectors
    const labelInput = page
      .getByText("Labels", { exact: true })
      .locator("..")
      .locator("input");
    const editorArea = page.getByText("Labels", { exact: true }).locator("..").locator("..");
    await expect(editorArea.getByText("platform-v2")).toBeVisible();
    await expect(editorArea.getByText("auth", { exact: true })).toBeVisible();

    // Remove the "auth" label
    await page.getByRole("button", { name: "Remove label auth" }).click();

    // Add the "infra" label
    await labelInput.fill("infra");
    const infraOption = page.locator("li").filter({ hasText: "infra" });
    await expect(infraOption).toBeVisible();
    await infraOption.click();

    // Close the dropdown by clicking the page heading (away from the picker)
    await page
      .getByRole("heading", { name: "Migrate authentication to OAuth2" })
      .click();

    // Save changes
    await page.getByRole("button", { name: "Save" }).click();

    // Verify updated labels
    // Scope to the label editor section to avoid matching labels inside ItemRow chips
    const labelSectionAfterSave = page.getByTestId("label-editor");
    await expect(labelSectionAfterSave.getByText("platform-v2")).toBeVisible();
    await expect(labelSectionAfterSave.getByText("infra")).toBeVisible();
    await expect(
      labelSectionAfterSave.getByText("auth", { exact: true })
    ).not.toBeVisible();
  });
});
