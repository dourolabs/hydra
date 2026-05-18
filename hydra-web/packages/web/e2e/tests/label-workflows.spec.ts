import { test, expect } from "../fixtures/auth";
import type { Page } from "@playwright/test";

// The Details rail tab (which renders the IssueLabelEditor) is not the
// default — Related is. Tests that operate on the label editor must first
// activate the Details rail tab.
async function openDetailsTab(page: Page): Promise<void> {
  await page.getByTestId("issue-rail-tab-details").click();
}

test.describe("Creating an issue with labels via LabelPicker @labels:create-with", () => {
  test("creates an issue with an existing and a new label @labels:create-with", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=all");
    await expect(page.getByText("Platform v2.0 Migration")).toBeVisible();

    // Open the create issue modal via the header dropdown
    await page.getByTestId("site-header-create").click();
    await page.getByTestId("site-header-new-issue").click();
    const modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();

    // Fill in required fields
    await modal.getByPlaceholder("Issue title…").fill("Test issue with labels");
    await modal
      .getByPlaceholder(/^Describe the issue/)
      .fill("E2E test for label creation workflow");

    // Open the labels picker (pill next to the "Labels" picker label)
    const labelsPill = modal
      .getByText("Labels", { exact: true })
      .locator("..")
      .locator("button[aria-expanded]")
      .first();
    await labelsPill.click();

    // The labels popover is portaled to document.body so its inputs and
    // option buttons are NOT inside the modal locator — use `page` scope.
    const search = page.getByPlaceholder("Search or create…");
    await expect(search).toBeVisible();
    await search.fill("plat");
    const existingOption = page.getByRole("button", { name: "platform-v2", exact: true });
    await expect(existingOption).toBeVisible();
    await existingOption.click();

    // Re-open the popover (it closed on selection? actually it stays open per
    // the picker's onMousedown outside check — but selection clears search).
    // Search input remains focused; type a name that doesn't exist.
    await search.fill("e2e-test-label");
    const createOption = page.getByRole("button", {
      name: /Create.*e2e-test-label/,
    });
    await expect(createOption).toBeVisible();
    await createOption.click();

    // Close the popover by clicking the title field (away from the picker)
    await modal.getByPlaceholder("Issue title…").click();

    // Verify both labels appear as chips in the pill summary
    await expect(labelsPill.getByText("platform-v2", { exact: true })).toBeVisible();
    await expect(labelsPill.getByText("e2e-test-label", { exact: true })).toBeVisible();

    // Submit the form
    await modal.getByRole("button", { name: "Create issue" }).click();

    // Modal should close
    await expect(modal).not.toBeVisible();

    // Success toast should appear
    await expect(page.getByText(/Issue .+ created/)).toBeVisible();
  });
});

test.describe("Hidden labels are excluded from user-facing label UI @labels:hidden", () => {
  // i-seed00008 ("Add dark mode support") has visible "platform-v2" and hidden "inbox" labels
  test("hidden labels do not appear in issue detail label display @labels:hidden", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00008");
    await expect(
      page.getByRole("heading", { name: "Add dark mode support" })
    ).toBeVisible();
    await openDetailsTab(page);

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
    await openDetailsTab(page);

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
    await openDetailsTab(page);

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

  test("hidden labels do not appear in label picker dropdown @labels:hidden", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=all");
    await expect(page.getByText("Platform v2.0 Migration")).toBeVisible();

    // Open the create issue modal via the header dropdown
    await page.getByTestId("site-header-create").click();
    await page.getByTestId("site-header-new-issue").click();
    const modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();

    // Open the labels picker pill
    const labelsPill = modal
      .getByText("Labels", { exact: true })
      .locator("..")
      .locator("button[aria-expanded]")
      .first();
    await labelsPill.click();

    // The labels popover is portaled to document.body — use `page` scope.
    await page.getByPlaceholder("Search or create…").fill("inbox");

    // Should show "Create" option but not an existing "inbox" label row
    const inboxOption = page.getByRole("button", { name: "inbox", exact: true });
    await expect(inboxOption).not.toBeVisible();
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
    await openDetailsTab(page);

    // i-seed00002 has labels "platform-v2" and "auth"
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
    await openDetailsTab(page);

    // Enter edit mode
    await page.getByRole("button", { name: "Edit labels" }).click();

    // Verify existing labels are shown as editable chips
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
    const labelSectionAfterSave = page.getByTestId("label-editor");
    await expect(labelSectionAfterSave.getByText("platform-v2")).toBeVisible();
    await expect(labelSectionAfterSave.getByText("infra")).toBeVisible();
    await expect(
      labelSectionAfterSave.getByText("auth", { exact: true })
    ).not.toBeVisible();
  });
});
