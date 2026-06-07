import { test, expect } from "../fixtures/auth";

// Issues list is mounted at `/` (router index route), not `/issues`.
const ISSUES_PATH = "/";

// From seed.json: c-seed00001 refers-to six issues plus one document. The
// client must bucket out the non-`i-` target before issuing
// `listIssues?ids=`. See PR-1 ([[i-kgmmbxrm]]).
const C1_REFERS_TO_ISSUES = [
  "i-seed00001",
  "i-seed00002",
  "i-seed00005",
  "i-seed00006",
  "i-seed00013",
  "i-seed00017",
];

// c-seed00004 refers-to two issues; i-seed00013 is already in the c-seed00001
// set, so adding c-seed00004 to the chip introduces exactly one new row
// (i-seed00014) for the union assertion below.
const C4_NEW_ISSUE = "i-seed00014";

test.describe("Issues page filter-by-relation", () => {
  test("FilterBar → Related chat narrows the list to chat→issue refers-to targets, URL persists, reload rehydrates @issues:filter-related-chat-narrows", async ({
    authenticatedPage: page,
  }) => {
    // Capture every listIssues request so we can prove the resolver bucketed
    // out non-`i-` ids before the request went out. PR-1's bug surfaced as
    // an `ids=…d-seed00006…` CSV that a strict backend rejected with 400.
    const listIssuesRequests: URL[] = [];
    page.on("request", (req) => {
      const url = new URL(req.url());
      if (url.pathname === "/api/v1/issues") {
        listIssuesRequests.push(url);
      }
    });

    await page.goto(ISSUES_PATH);
    await expect(page.getByTestId("filter-bar-add")).toBeVisible();

    // Open + Filter → pick Related chat. The picker opens automatically on
    // the freshly-added chip.
    await page.getByTestId("filter-bar-add").click();
    await expect(page.getByTestId("add-filter-menu")).toBeVisible();
    await page.getByTestId("add-filter-relatedChat").click();

    await expect(page.getByTestId("filter-chip-relatedChat")).toBeVisible();
    await expect(page.getByTestId("value-picker-relatedChat")).toBeVisible();

    // Pick c-seed00001. URL reflects the chip.
    await page.getByTestId("value-option-c-seed00001").click();
    await expect(page).toHaveURL(/[?&]relatedChat=c-seed00001\b/);

    // The narrowed rows are exactly the six issues the seed says c-seed00001
    // refers-to. d-seed00006 (a document) must not surface as a row. The
    // initial unfiltered paint shows all seeded issues; `toHaveCount` polls
    // until the filtered render replaces it before per-row checks.
    const rowSelector = page.locator(
      'tbody tr[data-testid^="issues-list-row-"]',
    );
    await expect(rowSelector).toHaveCount(C1_REFERS_TO_ISSUES.length);
    for (const id of C1_REFERS_TO_ISSUES) {
      await expect(page.getByTestId(`issues-list-row-${id}`)).toBeVisible();
    }

    // Every listIssues request that carried `ids=` must contain only
    // i-prefixed ids — this is the PR-1 fix.
    const narrowing = listIssuesRequests.filter((u) =>
      u.searchParams.has("ids"),
    );
    expect(narrowing.length).toBeGreaterThan(0);
    for (const url of narrowing) {
      const ids = (url.searchParams.get("ids") ?? "").split(",").filter(Boolean);
      expect(ids.length).toBeGreaterThan(0);
      for (const id of ids) {
        expect(id).toMatch(/^i-/);
      }
    }

    // Reload — chip + narrowed list rehydrate from the URL.
    await page.reload();
    await expect(page).toHaveURL(/[?&]relatedChat=c-seed00001\b/);
    await expect(page.getByTestId("filter-chip-relatedChat")).toBeVisible();
    for (const id of C1_REFERS_TO_ISSUES) {
      await expect(page.getByTestId(`issues-list-row-${id}`)).toBeVisible();
    }
  });

  test("changing the chip's selection keeps the previous rows visible until the new resolution lands — no flash @issues:filter-related-chat-no-flash", async ({
    authenticatedPage: page,
  }) => {
    // Hold the SECOND `/v1/relations` call (the one that fires after the user
    // adds c-seed00004 to the chip). The first call (c-seed00001 alone)
    // resolves normally and seeds the initial render. We test "switch by
    // adding a value" rather than "deselect, then select" because the
    // multi-select chip UI doesn't support an atomic replace — a
    // deselect-then-select sequence has an intermediate empty-values state
    // that legitimately drops the relation filter, which would mask the
    // no-flash invariant we care about.
    let releaseSecondRelations: (() => void) | null = null;
    let combinedSourceCalls = 0;
    await page.route(/\/api\/v1\/relations(\?|$)/, async (route) => {
      const url = new URL(route.request().url());
      const sourceIds = url.searchParams.get("source_ids") ?? "";
      if (
        url.searchParams.get("rel_type") === "refers-to" &&
        sourceIds.includes("c-seed00001") &&
        sourceIds.includes("c-seed00004")
      ) {
        combinedSourceCalls += 1;
        if (combinedSourceCalls === 1) {
          await new Promise<void>((resolve) => {
            releaseSecondRelations = resolve;
          });
        }
      }
      await route.continue();
    });

    // Land on the page with the relation chip already rehydrated. The first
    // resolver call (c-seed00001) is NOT intercepted by the gate above so it
    // resolves normally and the initial render is the c-seed00001 set.
    await page.goto(`${ISSUES_PATH}?relatedChat=c-seed00001`);
    await expect(page.getByTestId("filter-chip-relatedChat")).toBeVisible();
    for (const id of C1_REFERS_TO_ISSUES) {
      await expect(page.getByTestId(`issues-list-row-${id}`)).toBeVisible();
    }

    // Open the chip's value picker.
    await page
      .getByTestId("filter-chip-relatedChat")
      .getByRole("button", { name: /click to edit/i })
      .click();
    await expect(page.getByTestId("value-picker-relatedChat")).toBeVisible();

    // Add c-seed00004 to the existing selection. The chip's values become
    // [c-seed00001, c-seed00004], which triggers a new relation lookup with
    // both source ids. Our route intercept holds that lookup mid-flight.
    await page.getByTestId("value-option-c-seed00004").click();

    // Wait until our intercept has captured the in-flight call. This is a
    // condition wait (no wall-clock sleep): poll the side-effect counter.
    await expect.poll(() => combinedSourceCalls).toBeGreaterThan(0);

    // While the resolver is held, the rows MUST persist. Take a handful of
    // DOM observations in tight succession; if PR-2 regressed and the page
    // started forwarding `relationsLoading` into the view's `isLoading`,
    // the rows container would empty and the skeleton would appear.
    const rowSelector = page.locator(
      'tbody tr[data-testid^="issues-list-row-"]',
    );
    const observations: number[] = [];
    for (let i = 0; i < 6; i += 1) {
      observations.push(await rowSelector.count());
    }
    expect(Math.min(...observations)).toBe(C1_REFERS_TO_ISSUES.length);
    await expect(page.getByText("Loading issues…")).toHaveCount(0);
    await expect(
      page.getByText("No issues match the current filters."),
    ).toHaveCount(0);

    // Release the held call. The new union of c-seed00001 + c-seed00004
    // refers-to lands; the URL has tracked the swap throughout.
    expect(releaseSecondRelations).not.toBeNull();
    releaseSecondRelations?.();

    await expect(page).toHaveURL(
      /[?&]relatedChat=c-seed00001(?:%2C|,)c-seed00004\b/,
    );
    for (const id of C1_REFERS_TO_ISSUES) {
      await expect(page.getByTestId(`issues-list-row-${id}`)).toBeVisible();
    }
    await expect(
      page.getByTestId(`issues-list-row-${C4_NEW_ISSUE}`),
    ).toBeVisible();
  });
});
