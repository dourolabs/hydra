import { test, expect } from "../fixtures/auth";

test.describe("Documents @documents:list @documents:view-detail", () => {
  test("displays the documents list page @documents:list", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/documents");
    // Seed data documents should be visible
    await expect(
      page.getByText("ADR-001: OAuth2 Migration Strategy")
    ).toBeVisible();
    await expect(
      page.getByText("API Reference Guide v2.0")
    ).toBeVisible();
    await expect(
      page.getByText("Developer Onboarding Guide")
    ).toBeVisible();
  });

  test("can navigate to a document detail page @documents:view-detail", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/documents");
    await expect(
      page.getByText("ADR-001: OAuth2 Migration Strategy")
    ).toBeVisible();

    // Click on the first document to navigate to its detail page
    await page.getByText("ADR-001: OAuth2 Migration Strategy").click();
    await expect(page).toHaveURL(/\/documents\/d-seed00001/);

    // Verify the document detail page shows the title
    await expect(
      page.getByRole("heading", { name: "ADR-001: OAuth2 Migration Strategy" }).first()
    ).toBeVisible();
  });

  test("document detail page shows content and metadata @documents:view-detail", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/documents/d-seed00001");
    await expect(
      page.getByRole("heading", { name: "ADR-001: OAuth2 Migration Strategy" }).first()
    ).toBeVisible();

    // Verify path metadata is displayed
    await expect(
      page.getByText("/research/adr-001-oauth2-migration")
    ).toBeVisible();
  });

  test("document detail breadcrumb links back to documents list @documents:view-detail", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/documents/d-seed00001");
    const breadcrumb = page.locator('nav[aria-label="Breadcrumb"]');
    await expect(breadcrumb.getByText("Documents")).toBeVisible();

    // Click breadcrumb to go back to documents list
    await breadcrumb.getByText("Documents").click();
    await expect(page).toHaveURL(/\/documents$/);
  });
});
