import { test, expect } from "../fixtures/auth";

test.describe("Patches @patches:view-detail @patches:navigate", () => {
  test("displays patch detail page with title and status @patches:view-detail", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/patches/p-seed00001");
    await expect(
      page.getByRole("heading", { name: "Add OAuth2 provider integration" })
    ).toBeVisible();
  });

  test("patch detail page shows metadata tab with patch ID @patches:view-detail", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/patches/p-seed00001");
    await expect(
      page.getByRole("heading", { name: "Add OAuth2 provider integration" })
    ).toBeVisible();

    // Navigate to Metadata tab
    await page.getByRole("tab", { name: "Metadata" }).click();
    await expect(
      page.getByText("p-seed00001", { exact: true })
    ).toBeVisible();
  });

  test("can navigate to a patch from an issue's Patches tab @patches:navigate", async ({
    authenticatedPage: page,
  }) => {
    // i-seed00002 has patch p-seed00001
    await page.goto("/issues/i-seed00002");
    await expect(
      page.getByRole("heading", { name: "Migrate authentication to OAuth2" })
    ).toBeVisible();

    // Click on the Patches tab
    await page.getByRole("tab", { name: "Patches" }).click();

    // Click the patch link to navigate to it (use first() since the patch
    // appears in both PatchPreview and the Patches tab list)
    await page.getByText("p-seed00001").first().click();
    await expect(page).toHaveURL(/\/patches\/p-seed00001/);
    await expect(
      page.getByRole("heading", { name: "Add OAuth2 provider integration" })
    ).toBeVisible();
  });
});
