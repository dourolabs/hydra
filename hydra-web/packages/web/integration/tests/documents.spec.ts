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

    // Verify the document detail page shows the title.
    await expect(
      page.getByRole("heading", { name: "ADR-001: OAuth2 Migration Strategy" }).first(),
    ).toBeVisible();
    // Breadcrumb also surfaces the title (the chrome-side fallback on mobile,
    // where the page H1 collapses).
    const breadcrumb = page.locator('nav[aria-label="Breadcrumb"]');
    await expect(
      breadcrumb.getByText("ADR-001: OAuth2 Migration Strategy"),
    ).toBeVisible();
  });

  test("document detail page shows title and body @documents:view-detail", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/documents/d-seed00001");
    await expect(
      page.getByRole("heading", { name: "ADR-001: OAuth2 Migration Strategy" }).first(),
    ).toBeVisible();

    // Edit affordance is rendered (floats over the body at top-right).
    await expect(page.getByTestId("document-edit-button")).toBeVisible();
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

  test("mobile viewport hides the left tree and shows only the reader pane @mobile:documents-single-pane", async ({
    authenticatedPage: page,
  }) => {
    await page.setViewportSize({ width: 375, height: 812 });
    // The mobile sidebar drawer is open by default and intercepts pointer
    // events on the page content. Persist "hidden" before navigation so the
    // drawer stays closed for the in-pane click below.
    await page.addInitScript(() => {
      window.localStorage.setItem("hydra-sidebar-hidden", "1");
    });
    await page.goto("/documents");

    const main = page.locator("main");
    const tree = main.locator('aside[aria-label="Document tree"]');
    const reader = main.getByTestId("documents-reader-pane");

    // Reader pane must be visible on mobile.
    await expect(reader).toBeVisible();

    // Left tree must not be visible on mobile (CSS display:none collapses it).
    await expect(tree).toHaveCSS("display", "none");

    // Navigation up/down still works inside the reader pane: click a
    // subfolder row to navigate into it. Assert via a file row (the
    // `.breadcrumb` meta summary is display:none below 768px).
    const researchRow = reader.getByRole("button", { name: /research/ });
    await researchRow.click();
    await expect(
      reader.getByRole("link", { name: /ADR-001: OAuth2 Migration Strategy/ }),
    ).toBeVisible();
  });

  test("reader pane shows an up-one-level entry that navigates to the parent folder @documents:up-one-level", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/documents");

    const main = page.locator("main");
    const reader = main.getByTestId("documents-reader-pane");

    // At root, the up-one-level entry must NOT be rendered.
    await expect(reader.getByTestId("documents-up-one-level")).toHaveCount(0);

    // Navigate into /research via the left tree.
    const researchFolder = main.getByRole("treeitem", { name: /research/ }).first();
    await researchFolder.click({ position: { x: 80, y: 14 } });
    await expect(reader.getByText(/3 files · 0 folders/)).toBeVisible();

    // The up-one-level entry is now visible and labelled with the parent name.
    // Parent of /research is the root, so the label is "Up to /".
    const upEntry = reader.getByTestId("documents-up-one-level");
    await expect(upEntry).toBeVisible();
    await expect(upEntry).toContainText(/Up to \//);

    // Clicking it returns to the root path: the breadcrumb trail is empty
    // (no `.crumbCurrent` element rendered) and the file/folder counts reflect
    // the root listing.
    await upEntry.click();
    await expect(reader.getByText(/\d+ files? · \d+ folders?/)).toBeVisible();
    await expect(reader.getByTestId("documents-up-one-level")).toHaveCount(0);
  });
});
