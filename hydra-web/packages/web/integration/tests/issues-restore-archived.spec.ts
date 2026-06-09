import { test, expect } from "../fixtures/auth";

const ISSUES_PATH = "/";

// Same fixture issue the include-archived filter spec targets. Stable across
// `dev/reset` because the seeded JSON is reloaded on each test.
const TARGET_ISSUE = "i-seed00003";

async function archiveIssue(id: string): Promise<void> {
  const res = await fetch(`http://localhost:8080/v1/issues/${id}`, {
    method: "DELETE",
    headers: { Authorization: "Bearer dev-token-12345" },
  });
  if (!res.ok) {
    throw new Error(`archive setup failed: ${res.status} ${res.statusText}`);
  }
}

test.describe("Issues page row-level 'Restore' action @issues:restore-archived", () => {
  test("clicking Restore on an archived row unarchives it and the row drops from the include-archived view @issues:restore-archived", async ({
    authenticatedPage: page,
  }) => {
    await archiveIssue(TARGET_ISSUE);

    const updateBodies: Array<{ url: URL; body: string }> = [];
    page.on("request", (req) => {
      const url = new URL(req.url());
      if (
        req.method() === "PUT" &&
        url.pathname === `/api/v1/issues/${TARGET_ISSUE}`
      ) {
        updateBodies.push({ url, body: req.postData() ?? "" });
      }
    });

    await page.goto(`${ISSUES_PATH}?includeArchived=1`);
    await expect(page.getByTestId("filter-chip-includeArchived")).toBeVisible();

    const archivedRow = page.getByTestId(`issues-list-row-${TARGET_ISSUE}`);
    await expect(archivedRow).toBeVisible();
    await expect(archivedRow).toHaveAttribute("data-archived", "true");

    // Hover reveals the Restore button. Always-on visibility for the action
    // would clutter live rows, so the row reveals on hover (mirroring
    // DocumentRow's hover-revealed Delete).
    await archivedRow.hover();
    const restoreBtn = page.getByTestId(`issues-row-restore-${TARGET_ISSUE}`);
    await expect(restoreBtn).toBeVisible();

    await restoreBtn.click();

    // A PUT to /v1/issues/:id should fire with `deleted: false` in the body.
    await expect
      .poll(() =>
        updateBodies.some(
          (u) =>
            u.url.pathname === `/api/v1/issues/${TARGET_ISSUE}` &&
            /"deleted"\s*:\s*false/.test(u.body),
        ),
      )
      .toBe(true);

    // The include-archived view continues to show the row (the chip is
    // still on), but after the list refetch it renders without the ARCHIVED
    // flag because the server now reports `deleted: false`.
    await expect(
      page.getByTestId(`issues-row-archived-${TARGET_ISSUE}`),
    ).toHaveCount(0);
    await expect(
      page.getByTestId(`issues-list-row-${TARGET_ISSUE}`),
    ).not.toHaveAttribute("data-archived", "true");

    // Drop the Include archived chip; the restored issue appears in the
    // default view.
    await page
      .getByTestId("filter-chip-includeArchived")
      .getByRole("button", { name: /remove include archived filter/i })
      .click();
    await expect(page).not.toHaveURL(/includeArchived/);

    await expect(
      page.getByTestId(`issues-list-row-${TARGET_ISSUE}`),
    ).toBeVisible();
    await expect(
      page.getByTestId(`issues-row-archived-${TARGET_ISSUE}`),
    ).toHaveCount(0);
  });
});
