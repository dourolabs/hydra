import { test, expect } from "../fixtures/auth";

// Same seeded issue used by the include-archived list/restore specs. Stable
// across `dev/reset` because the seeded JSON is reloaded on each test.
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

test.describe("Issue Detail page for archived issues @issues:view-detail-archived", () => {
  test("renders the page with an Archived badge when the issue is soft-deleted @issues:view-detail-archived", async ({
    authenticatedPage: page,
  }) => {
    await archiveIssue(TARGET_ISSUE);

    const getIssueUrls: URL[] = [];
    page.on("request", (req) => {
      const url = new URL(req.url());
      if (url.pathname === `/api/v1/issues/${TARGET_ISSUE}`) {
        getIssueUrls.push(url);
      }
    });

    await page.goto(`/issues/${TARGET_ISSUE}`);

    // The detail page renders normally: the seeded title is now the breadcrumb
    // tail (replacing the bare id) and the id chip stays visible inside the
    // detail body's title row (rather than a 404 / empty shell).
    await expect(
      page.locator('nav[aria-label="Breadcrumb"]').getByText(
        "Implement OAuth2 token refresh logic",
      ),
    ).toBeVisible();
    await expect(
      page.getByRole("main").getByText(TARGET_ISSUE, { exact: true }),
    ).toBeVisible();

    // The Archived badge appears in the title row.
    await expect(page.getByTestId("issue-archived-badge")).toBeVisible();
    await expect(page.getByTestId("issue-archived-badge")).toHaveText(/Archived/i);

    // The GET against the issue must have carried `include_deleted=true` —
    // otherwise the mock server would 404 on a soft-deleted row.
    expect(
      getIssueUrls.some(
        (u) => u.searchParams.get("include_deleted") === "true",
      ),
    ).toBe(true);
  });

  test("does not render the Archived badge for live issues @issues:view-detail-archived", async ({
    authenticatedPage: page,
  }) => {
    // i-seed00002 is a live (non-deleted) issue from the seed fixture.
    await page.goto("/issues/i-seed00002");

    await expect(
      page.locator('nav[aria-label="Breadcrumb"]').getByText(
        "Migrate authentication to OAuth2",
      ),
    ).toBeVisible();
    await expect(page.getByTestId("issue-archived-badge")).toHaveCount(0);
  });
});
