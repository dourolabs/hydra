import { test, expect } from "../fixtures/auth";

const ISSUES_PATH = "/";

// Pick a seeded issue id we know exists in the unfiltered list, then soft-delete
// it via the mock API before the page loads. The chosen id is stable across
// resets because the fixture is the same JSON file on every `dev/reset`.
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

test.describe("Issues page 'Include archived' filter @issues:filter-include-archived", () => {
  test("toggling the chip shows soft-deleted rows and adds include_deleted to the request @issues:filter-include-archived", async ({
    authenticatedPage: page,
  }) => {
    await archiveIssue(TARGET_ISSUE);

    const listIssuesUrls: URL[] = [];
    page.on("request", (req) => {
      const url = new URL(req.url());
      if (url.pathname === "/api/v1/issues") {
        listIssuesUrls.push(url);
      }
    });

    await page.goto(ISSUES_PATH);
    await expect(page.getByTestId("filter-bar-add")).toBeVisible();

    // Default state: archived row is hidden. Wait for the initial list paint
    // so the negative assertion is meaningful (and not just "row hasn't
    // rendered yet").
    const someRow = page.locator('tbody tr[data-testid^="issues-list-row-"]').first();
    await expect(someRow).toBeVisible();
    await expect(
      page.getByTestId(`issues-list-row-${TARGET_ISSUE}`),
    ).toHaveCount(0);

    // No request so far should have set `include_deleted=true`.
    expect(
      listIssuesUrls.every(
        (u) => u.searchParams.get("include_deleted") !== "true",
      ),
    ).toBe(true);

    // Open + Filter → pick Include archived. The presence chip lands on the
    // bar; no value-picker should open for it.
    await page.getByTestId("filter-bar-add").click();
    await expect(page.getByTestId("add-filter-menu")).toBeVisible();
    await page.getByTestId("add-filter-includeArchived").click();

    await expect(page.getByTestId("filter-chip-includeArchived")).toBeVisible();
    await expect(page.getByTestId("value-picker-includeArchived")).toHaveCount(0);

    // URL persists the presence flag as `?includeArchived=1`.
    await expect(page).toHaveURL(/[?&]includeArchived=1\b/);

    // A subsequent listIssues call must carry `include_deleted=true`.
    await expect
      .poll(() =>
        listIssuesUrls.some(
          (u) => u.searchParams.get("include_deleted") === "true",
        ),
      )
      .toBe(true);

    // The archived row is now visible, with the ARCHIVED tag.
    const archivedRow = page.getByTestId(`issues-list-row-${TARGET_ISSUE}`);
    await expect(archivedRow).toBeVisible();
    await expect(archivedRow).toHaveAttribute("data-archived", "true");
    await expect(
      page.getByTestId(`issues-row-archived-${TARGET_ISSUE}`),
    ).toBeVisible();

    // Dismiss the chip. URL drops the flag and no further listIssues call
    // carries `include_deleted`. The dismissed-state page may render from
    // react-query's cache (the unfiltered key was warmed on first paint),
    // so we assert the absence of stale flags rather than waiting for a
    // specific fresh fetch — the user-visible "archived row hides" check
    // below covers the rendering contract.
    const baselineIssuesCount = listIssuesUrls.length;
    await page
      .getByTestId("filter-chip-includeArchived")
      .getByRole("button", { name: /remove include archived filter/i })
      .click();
    await expect(page).not.toHaveURL(/includeArchived/);
    await expect
      .poll(() =>
        listIssuesUrls
          .slice(baselineIssuesCount)
          .every((u) => u.searchParams.get("include_deleted") !== "true"),
      )
      .toBe(true);

    // Archived row hides again.
    await expect(
      page.getByTestId(`issues-list-row-${TARGET_ISSUE}`),
    ).toHaveCount(0);
  });

  test("?includeArchived=1 rehydrates the chip on first paint @issues:filter-include-archived-rehydrate", async ({
    authenticatedPage: page,
  }) => {
    await archiveIssue(TARGET_ISSUE);

    const listIssuesUrls: URL[] = [];
    page.on("request", (req) => {
      const url = new URL(req.url());
      if (url.pathname === "/api/v1/issues") {
        listIssuesUrls.push(url);
      }
    });

    await page.goto(`${ISSUES_PATH}?includeArchived=1`);
    await expect(page.getByTestId("filter-chip-includeArchived")).toBeVisible();
    await expect(
      page.getByTestId(`issues-list-row-${TARGET_ISSUE}`),
    ).toBeVisible();

    // The first listIssues request after rehydration must carry the flag.
    await expect
      .poll(() =>
        listIssuesUrls.some(
          (u) => u.searchParams.get("include_deleted") === "true",
        ),
      )
      .toBe(true);
  });
});
