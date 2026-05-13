import { test, expect } from "../fixtures/auth";

test.describe("Sidebar Documents @sidebar:documents", () => {
  test("expanding a top-level folder reveals leaf documents and clicking one navigates @sidebar:documents", async ({
    authenticatedPage: page,
  }) => {
    // Start somewhere with the sidebar visible.
    await page.goto("/");

    // The Documents section should be expanded by default and the tree should
    // render the top-level folders from the seed data (/research and /docs).
    const researchFolder = page.getByTestId(
      "sidebar-doc-tree-folder-/research",
    );
    await expect(researchFolder).toBeVisible();

    // Folder starts collapsed.
    await expect(researchFolder).toHaveAttribute("aria-expanded", "false");

    // Expand /research; its child documents should appear as leaves.
    await researchFolder.click();
    await expect(researchFolder).toHaveAttribute("aria-expanded", "true");

    const leaf = page.getByTestId("sidebar-doc-tree-leaf-d-seed00001");
    await expect(leaf).toBeVisible();

    // Clicking the leaf navigates to that document's detail page.
    await leaf.click();
    await expect(page).toHaveURL(/\/documents\/d-seed00001/);
  });

  test("Documents 'More' link navigates to /documents @sidebar:documents", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/");
    await page.getByTestId("sidebar-section-documents-more").click();
    await expect(page).toHaveURL(/\/documents$/);
  });
});
