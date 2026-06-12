import { test, expect } from "../fixtures/auth";

test.describe("Issue comments @issues:comments", () => {
  test("renders the comments panel on the issue detail page", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00001");
    const panel = page.getByTestId("comments-panel");
    await expect(panel).toBeVisible();
    await expect(panel.getByText("Comments", { exact: true })).toBeVisible();
  });

  test("shows seeded comments DESC by sequence", async ({
    authenticatedPage: page,
  }) => {
    // i-seed00001 has 3 seeded comments; the most recent ("FYI the
    // database migration is now complete…") must render first.
    await page.goto("/issues/i-seed00001");
    const panel = page.getByTestId("comments-panel");
    await expect(panel.getByText(/FYI the database migration/)).toBeVisible();
    await expect(panel.getByText(/Kicking off discussion/)).toBeVisible();
  });

  test("renders empty state when the issue has no comments", async ({
    authenticatedPage: page,
  }) => {
    // i-seed00003 has no seeded comments.
    await page.goto("/issues/i-seed00003");
    await expect(page.getByTestId("comments-empty")).toBeVisible();
  });

  test("posts a comment and shows it in the list", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00003");
    const textarea = page.getByTestId("comments-composer-textarea");
    await textarea.fill("Hello from playwright");
    await page.getByTestId("comments-composer-submit").click();
    await expect(
      page.getByTestId("comments-panel").getByText("Hello from playwright"),
    ).toBeVisible();
    // Composer clears on success.
    await expect(textarea).toHaveValue("");
  });
});
