import { test, expect } from "../fixtures/auth";

// A live (non-archived) seed issue. `dev/reset` reloads the fixture between
// tests so this row is always present before the test runs.
const TARGET_ISSUE = "i-seed00002";

test.describe("Manual archive action @issues:archive", () => {
  test("clicking Archive on the issue detail page flips the page to its archived rendering @issues:archive", async ({
    authenticatedPage: page,
  }) => {
    const deleteUrls: URL[] = [];
    page.on("request", (req) => {
      const url = new URL(req.url());
      if (
        req.method() === "DELETE" &&
        url.pathname === `/api/v1/issues/${TARGET_ISSUE}`
      ) {
        deleteUrls.push(url);
      }
    });

    await page.goto(`/issues/${TARGET_ISSUE}`);

    // Pre-archive: the badge is absent and the Archive button is present.
    await expect(page.getByTestId("issue-archived-badge")).toHaveCount(0);
    const archiveBtn = page.getByTestId("issue-detail-archive");
    await expect(archiveBtn).toBeVisible();

    await archiveBtn.click();

    // The DELETE goes out.
    await expect
      .poll(() =>
        deleteUrls.some(
          (u) => u.pathname === `/api/v1/issues/${TARGET_ISSUE}`,
        ),
      )
      .toBe(true);

    // The Archived badge appears in the title row and the Archive button
    // disappears (a second click would be a no-op against the same row).
    await expect(page.getByTestId("issue-archived-badge")).toBeVisible();
    await expect(page.getByTestId("issue-detail-archive")).toHaveCount(0);
  });
});
