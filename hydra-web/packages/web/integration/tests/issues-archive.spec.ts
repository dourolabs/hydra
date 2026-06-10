import { test, expect } from "../fixtures/auth";

// A live (non-archived) seed issue. `dev/reset` reloads the fixture between
// tests so this row is always present before the test runs.
const TARGET_ISSUE = "i-seed00002";

test.describe("Manual archive action @issues:archive", () => {
  test("clicking Archive on a non-archived row drops it from the default view before the DELETE confirms @issues:archive", async ({
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

    // Stall the DELETE response until released so the row must drop from
    // the optimistic cache update, not the server round-trip.
    let releaseDelete!: () => void;
    const deleteReleased = new Promise<void>((r) => (releaseDelete = r));
    await page.route(
      `**/api/v1/issues/${TARGET_ISSUE}`,
      async (route, req) => {
        if (req.method() !== "DELETE") {
          await route.fallback();
          return;
        }
        await deleteReleased;
        await route.fallback();
      },
    );

    await page.goto("/");

    const row = page.getByTestId(`issues-list-row-${TARGET_ISSUE}`);
    await expect(row).toBeVisible();

    // Archive is hover-revealed (mirrors the Restore pattern).
    await row.hover();
    const archiveBtn = page.getByTestId(`issues-row-archive-${TARGET_ISSUE}`);
    await expect(archiveBtn).toBeVisible();

    await archiveBtn.click();

    // While the DELETE is still in flight, the row must already be gone
    // from the default view via the optimistic paginated-cache update.
    await expect(
      page.getByTestId(`issues-list-row-${TARGET_ISSUE}`),
    ).toHaveCount(0);

    releaseDelete();

    // A DELETE against /v1/issues/:id must have fired.
    await expect
      .poll(() =>
        deleteUrls.some(
          (u) => u.pathname === `/api/v1/issues/${TARGET_ISSUE}`,
        ),
      )
      .toBe(true);

    // After the server confirms and the list refetches, the row is still
    // absent from the default view.
    await expect(
      page.getByTestId(`issues-list-row-${TARGET_ISSUE}`),
    ).toHaveCount(0);

    // Flipping the Include archived chip re-surfaces the now-archived row
    // with its ARCHIVED tag.
    await page.goto("/?includeArchived=1");
    await expect(
      page.getByTestId(`issues-list-row-${TARGET_ISSUE}`),
    ).toBeVisible();
    await expect(
      page.getByTestId(`issues-row-archived-${TARGET_ISSUE}`),
    ).toBeVisible();
  });

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
