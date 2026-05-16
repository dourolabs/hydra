import { test, expect } from "../fixtures/auth";

test.describe("Documents @documents:list @documents:view-detail", () => {
  test("displays the documents list page with tree explorer @documents:list", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/documents");

    // Scope assertions to the main page (the sidebar also renders a Documents tree).
    const main = page.locator("main");

    // Top-level path folders should be visible and expanded by default
    await expect(main.getByRole("button", { name: /research/ })).toBeVisible();
    await expect(main.getByRole("button", { name: /docs/ })).toBeVisible();

    // Top-level folders are expanded by default, and leaf documents render directly as DocumentRows
    await expect(main.getByText("ADR-001: OAuth2 Migration Strategy")).toBeVisible();
  });

  test("can navigate to a document detail page @documents:view-detail", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/documents");

    // Scope to the main page; the sidebar tree also has links to documents.
    const main = page.locator("main");

    // Top-level folders are expanded by default; leaf documents render directly as DocumentRows
    await expect(main.getByText("ADR-001: OAuth2 Migration Strategy")).toBeVisible();

    // Click on the document link to navigate to its detail page
    await main.getByText("ADR-001: OAuth2 Migration Strategy").click();
    await expect(page).toHaveURL(/\/documents\/d-seed00001/);

    // Verify the document detail page shows the title
    await expect(
      page.getByRole("heading", { name: "ADR-001: OAuth2 Migration Strategy" }).first(),
    ).toBeVisible();
  });

  test("document detail page shows content and metadata @documents:view-detail", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/documents/d-seed00001");
    await expect(
      page.getByRole("heading", { name: "ADR-001: OAuth2 Migration Strategy" }).first(),
    ).toBeVisible();

    // Verify path metadata is displayed
    await expect(page.getByText("/research/adr-001-oauth2-migration")).toBeVisible();
  });

  test("clicking a folder shows its documents in the right pane @documents:list", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/documents");

    const main = page.locator("main");

    // Click the "research" folder in the left tree. Use a positional click past
    // the chevron (~14px wide) so the row's select handler fires instead of the
    // expand/collapse toggle.
    const researchFolder = main.getByRole("treeitem", { name: /research/ }).first();
    await researchFolder.click({ position: { x: 80, y: 14 } });

    // Right pane should now list documents whose path is an immediate child
    // of /research. The seed has three: ADR-001, ADR-002, GraphQL Federation.
    const readerPane = main.getByTestId("documents-reader-pane");
    await expect(readerPane.getByText(/3 files · 0 folders/)).toBeVisible();
    const adr1 = readerPane.getByRole("link", {
      name: /ADR-001: OAuth2 Migration Strategy/,
    });
    await expect(adr1).toBeVisible();
    await expect(
      readerPane.getByRole("link", {
        name: /ADR-002: Real-time Collaboration Architecture/,
      }),
    ).toBeVisible();
    await expect(
      readerPane.getByRole("link", {
        name: /GraphQL Federation Migration Plan/,
      }),
    ).toBeVisible();

    // Clicking a doc row navigates to the document detail page.
    await adr1.click();
    await expect(page).toHaveURL(/\/documents\/d-seed00001/);
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
