import { test, expect } from "../fixtures/auth";

// Phase 4: archive-with-confirmation flow on `StatusSettingsModal`. The
// modal's old bulk-move-then-delete UX is gone; the backend cascade now
// archives every active issue at the to-archive status. The default
// project (`j-defaul`) has seeded issues at multiple statuses, so we
// drive the archive from a column whose count is known to be non-zero
// (`open`).

test.describe("Status settings — Archive status @projects:status-archive", () => {
  test("archive on a non-empty column shows N-issues confirmation and drops the column from the board on confirm @projects:status-archive", async ({
    authenticatedPage: page,
  }) => {
    const archiveCalls: string[] = [];
    page.on("request", (req) => {
      const url = new URL(req.url());
      if (
        req.method() === "POST" &&
        /\/api\/v1\/projects\/[^/]+\/statuses\/[^/]+\/archive$/.test(
          url.pathname,
        )
      ) {
        archiveCalls.push(url.pathname);
      }
    });

    await page.goto("/?selected=all");
    await page.getByTestId("issues-layout-board").click();

    // Sanity: the seed has a non-empty `open` column on the default project.
    await expect(page.getByTestId("board-col-default-open")).toBeVisible();

    // Gear opens StatusSettingsModal. The gear icon is hidden until column
    // hover, mirroring the add-issue button pattern.
    const openHead = page.getByTestId("board-col-head-default-open");
    await openHead.hover();
    await page.getByTestId("board-col-gear-default-open").click();

    const modal = page.getByRole("dialog", { name: /^Status — Open/ });
    await expect(modal).toBeVisible();

    // No bulk-move dropdown.
    await expect(
      modal.getByTestId("status-settings-move-target"),
    ).toHaveCount(0);
    await expect(modal.getByTestId("status-settings-delete")).toHaveCount(0);

    // Clicking Archive opens a separate confirmation dialog with the count.
    await modal.getByTestId("status-settings-archive").click();
    const confirmDialog = page.getByRole("dialog", { name: /Archive Status/ });
    await expect(confirmDialog).toBeVisible();
    await expect(confirmDialog).toContainText(/\d+ issue\(s\)/);
    await expect(confirmDialog).toContainText("archived");

    // Confirm — fires the per-status archive endpoint.
    await confirmDialog.getByRole("button", { name: "Archive" }).click();
    await expect(modal).toBeHidden();

    await expect
      .poll(() =>
        archiveCalls.some((p) =>
          p.endsWith("/projects/j-defaul/statuses/open/archive"),
        ),
      )
      .toBe(true);

    // The archived column no longer renders on the active board. The
    // other default-project columns stay in place.
    await expect(page.getByTestId("board-col-default-open")).toHaveCount(0);
    await expect(page.getByTestId("board-col-default-in-progress")).toBeVisible();
  });

  test("archive on an empty column shows a generic confirmation (no issue count) @projects:status-archive", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=all");
    await page.getByTestId("issues-layout-board").click();

    // The default project's `failed` column is empty in the seed — drive
    // the archive from there to verify the empty-column copy.
    const head = page.getByTestId("board-col-head-default-failed");
    await head.hover();
    await page.getByTestId("board-col-gear-default-failed").click();

    const modal = page.getByRole("dialog", { name: /^Status — Failed/ });
    await expect(modal).toBeVisible();
    await modal.getByTestId("status-settings-archive").click();

    const confirmDialog = page.getByRole("dialog", { name: /Archive Status/ });
    await expect(confirmDialog).toBeVisible();
    await expect(confirmDialog).not.toContainText("issue(s)");

    await confirmDialog.getByRole("button", { name: "Archive" }).click();
    await expect(modal).toBeHidden();
    await expect(page.getByTestId("board-col-default-failed")).toHaveCount(0);
  });
});
