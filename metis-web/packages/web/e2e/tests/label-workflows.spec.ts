import { test, expect } from "../fixtures/auth";

test.describe("Label display on dashboard item rows", () => {
  test("shows label chips on issues with labels", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=everything");
    await expect(page.getByText("Platform v2.0 Migration")).toBeVisible();

    // Seed data: i-seed00001 has "platform-v2", i-seed00002 has "platform-v2" + "auth",
    // i-seed00006 has "infra"
    const rows = page.locator("div[role=button]");
    await expect(rows.filter({ hasText: "platform-v2" }).first()).toBeVisible();
    await expect(rows.filter({ hasText: "auth" }).first()).toBeVisible();
    await expect(rows.filter({ hasText: "infra" }).first()).toBeVisible();
  });

  test("label chips appear within their respective issue rows", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=everything");
    await expect(page.getByText("Platform v2.0 Migration")).toBeVisible();

    // Verify "infra" label chip appears in the i-seed00006 row
    const rateRow = page.locator("div[role=button]").filter({
      hasText: "Implement API rate limiting",
    });
    await expect(rateRow.getByText("infra")).toBeVisible();

    // Verify "platform-v2" label chip appears in the i-seed00001 row
    const migrationRow = page.locator("div[role=button]").filter({
      hasText: "Platform v2.0 Migration",
    });
    await expect(migrationRow.getByText("platform-v2")).toBeVisible();
  });
});

test.describe("Creating an issue with labels via LabelPicker", () => {
  test("creates an issue with an existing and a new label", async ({
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

test.describe("Editing labels on issue detail page via IssueLabelEditor", () => {
  test.beforeEach(async () => {
    await fetch("http://localhost:8080/v1/dev/reset", {
      method: "POST",
      headers: { Authorization: "Bearer dev-token-12345" },
    });
  });

  test("displays existing labels on issue detail", async ({
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

  test("can add and remove labels via the editor", async ({
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
