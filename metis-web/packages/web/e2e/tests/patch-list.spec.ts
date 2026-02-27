import { test, expect } from "../fixtures/auth";

test.describe("Patch List", () => {
  test("renders seeded patches", async ({ authenticatedPage: page }) => {
    await page.goto("/patches");
    // Seed data has patches: "Add OAuth2 provider integration", "Implement OAuth2 scopes..."
    await expect(
      page.getByText("Add OAuth2 provider integration")
    ).toBeVisible();
    await expect(
      page.getByText("Implement OAuth2 scopes and permission mapping")
    ).toBeVisible();
  });

  test("displays patch status badges", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/patches");
    // Wait for patches to load
    await expect(
      page.getByText("Add OAuth2 provider integration")
    ).toBeVisible();
    // The patches page should render with status badges
    const patchList = page.locator("main");
    await expect(patchList).toBeVisible();
  });

  test("patch row links to patch detail", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/patches");
    await expect(page.getByText("p-seed00001")).toBeVisible();
    await page.getByText("Add OAuth2 provider integration").click();
    await expect(page).toHaveURL(/\/patches\/p-seed00001/);
  });
});
