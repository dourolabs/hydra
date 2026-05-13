import { test, expect } from "../fixtures/auth";

test.describe("Issue Create @issues:create", () => {
  test("can create a new issue via the UI @issues:create", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=all");
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
    await page.goto("/?selected=all");
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
    await page.goto("/?selected=all");
    await expect(page.getByText("Platform v2.0 Migration")).toBeVisible();

    // Open the create issue modal
    await page.getByRole("button", { name: "+ Create Issue" }).click();
    const modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();

    // Cancel the modal
    await modal.getByRole("button", { name: "Cancel" }).click();
    await expect(modal).not.toBeVisible();
  });

  test("clears draft when Cancel is clicked @issues:create", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=all");
    await expect(page.getByText("Platform v2.0 Migration")).toBeVisible();

    await page.getByRole("button", { name: "+ Create Issue" }).click();
    const modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();

    await modal
      .getByPlaceholder("Short summary (optional)")
      .fill("Cancel clears");
    await modal
      .getByPlaceholder("Describe the issue...")
      .fill("Should be cleared");

    await modal.getByRole("button", { name: "Cancel" }).click();
    await expect(modal).not.toBeVisible();

    // Reopen — fields should be empty.
    await page.getByRole("button", { name: "+ Create Issue" }).click();
    await expect(modal).toBeVisible();
    await expect(
      modal.getByPlaceholder("Short summary (optional)"),
    ).toHaveValue("");
    await expect(modal.getByPlaceholder("Describe the issue...")).toHaveValue(
      "",
    );
  });

  test("preserves draft on dismiss @issues:create", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=all");
    await expect(page.getByText("Platform v2.0 Migration")).toBeVisible();

    await page.getByRole("button", { name: "+ Create Issue" }).click();
    const modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();

    await modal
      .getByPlaceholder("Short summary (optional)")
      .fill("Preserved title");
    await modal
      .getByPlaceholder("Describe the issue...")
      .fill("Preserved description");

    // Dismiss via Escape (same path as backdrop click and header ✕).
    await page.keyboard.press("Escape");
    await expect(modal).not.toBeVisible();

    // Reopen — draft values should still be there.
    await page.getByRole("button", { name: "+ Create Issue" }).click();
    await expect(modal).toBeVisible();
    await expect(
      modal.getByPlaceholder("Short summary (optional)"),
    ).toHaveValue("Preserved title");
    await expect(modal.getByPlaceholder("Describe the issue...")).toHaveValue(
      "Preserved description",
    );
  });
});
