import { test, expect } from "../fixtures/auth";

test.describe("Issue Create @issues:create", () => {
  test("can create a new issue via the UI @issues:create", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=everything");
    await expect(page.getByText("Platform v2.0 Migration")).toBeVisible();

    // Open the create issue modal
    await page.getByRole("button", { name: "+ Create Issue" }).click();
    const modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();

    // Fill in the title
    await modal
      .getByPlaceholder("Short summary (optional)")
      .fill("E2E test issue creation");

    // Fill in the description (required)
    await modal
      .getByPlaceholder("Describe the issue...")
      .fill("This issue was created by an E2E test");

    // Submit the form
    await modal.getByRole("button", { name: "Create Issue" }).click();

    // Modal should close
    await expect(modal).not.toBeVisible();

    // Success toast should appear
    await expect(page.getByText(/Issue .+ created/)).toBeVisible();
  });

  test("create issue button is disabled when description is empty @issues:create", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=everything");
    await expect(page.getByText("Platform v2.0 Migration")).toBeVisible();

    // Open the create issue modal
    await page.getByRole("button", { name: "+ Create Issue" }).click();
    const modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();

    // Create button should be disabled when description is empty
    await expect(
      modal.getByRole("button", { name: "Create Issue" })
    ).toBeDisabled();

    // Fill only the title (description is required)
    await modal
      .getByPlaceholder("Short summary (optional)")
      .fill("Title only");

    // Button should still be disabled
    await expect(
      modal.getByRole("button", { name: "Create Issue" })
    ).toBeDisabled();
  });

  test("can cancel issue creation @issues:create", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=everything");
    await expect(page.getByText("Platform v2.0 Migration")).toBeVisible();

    // Open the create issue modal
    await page.getByRole("button", { name: "+ Create Issue" }).click();
    const modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();

    // Cancel the modal
    await modal.getByRole("button", { name: "Cancel" }).click();
    await expect(modal).not.toBeVisible();
  });
});
