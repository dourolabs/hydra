import { test, expect } from "../fixtures/auth";

// From seed.json: each top-level issue carries a `patches` array; the mock
// server derives `has-patch` relations from it on the fly
// (mock-server/src/routes/relations.ts:buildRelationsFromIssues).
//
// i-seed00002 → p-seed00001
// i-seed00004 → p-seed00002
const I2_HAS_PATCHES = ["p-seed00001"];
const I4_HAS_PATCHES = ["p-seed00002"];

test.describe("Patches page filter-by-relation", () => {
  test("FilterBar → Related issue narrows the list to has-patch targets, URL persists, reload rehydrates @patches:filter-related-issue-narrows", async ({
    authenticatedPage: page,
  }) => {
    const listPatchesRequests: URL[] = [];
    page.on("request", (req) => {
      const url = new URL(req.url());
      if (url.pathname === "/api/v1/patches") {
        listPatchesRequests.push(url);
      }
    });

    await page.goto("/patches");
    await expect(page.getByTestId("filter-bar-add")).toBeVisible();

    // + Filter → Related issue. The picker opens on the new chip.
    await page.getByTestId("filter-bar-add").click();
    await expect(page.getByTestId("add-filter-menu")).toBeVisible();
    await page.getByTestId("add-filter-relatedIssue").click();

    await expect(page.getByTestId("filter-chip-relatedIssue")).toBeVisible();
    await expect(page.getByTestId("value-picker-relatedIssue")).toBeVisible();

    // Pick two issues whose `has-patch` targets are distinct, so the union
    // emits two visible rows (each top-level issue carries exactly one patch
    // in the seed).
    await page.getByTestId("value-option-i-seed00002").click();
    await page.getByTestId("value-option-i-seed00004").click();
    await expect(page).toHaveURL(
      /[?&]relatedIssue=i-seed00002(?:%2C|,)i-seed00004\b/,
    );

    // The narrowed rows are exactly the union of has-patch targets. The
    // initial unfiltered paint shows all seeded patches; `toHaveCount` polls
    // until the filtered render replaces it before per-row checks.
    const expected = [...I2_HAS_PATCHES, ...I4_HAS_PATCHES];
    const rowSelector = page.locator(
      'tbody tr[data-testid^="patches-list-row-"]',
    );
    await expect(rowSelector).toHaveCount(expected.length);
    for (const id of expected) {
      await expect(page.getByTestId(`patches-list-row-${id}`)).toBeVisible();
    }

    // Every listPatches request that carried `ids=` must contain only
    // p-prefixed ids. The relation resolver for `relatedIssue` walks
    // `has-patch` outbound, which already yields patch ids, but assert this
    // anyway so a future regression in the resolver fires loudly.
    const narrowing = listPatchesRequests.filter((u) =>
      u.searchParams.has("ids"),
    );
    expect(narrowing.length).toBeGreaterThan(0);
    for (const url of narrowing) {
      const ids = (url.searchParams.get("ids") ?? "").split(",").filter(Boolean);
      expect(ids.length).toBeGreaterThan(0);
      for (const id of ids) {
        expect(id).toMatch(/^p-/);
      }
    }

    // Reload — chip + narrowed list rehydrate from the URL.
    await page.reload();
    await expect(page).toHaveURL(
      /[?&]relatedIssue=i-seed00002(?:%2C|,)i-seed00004\b/,
    );
    await expect(page.getByTestId("filter-chip-relatedIssue")).toBeVisible();
    for (const id of expected) {
      await expect(page.getByTestId(`patches-list-row-${id}`)).toBeVisible();
    }
  });

  test("changing the chip's selection keeps the previous rows visible until the new resolution lands — no flash @patches:filter-related-issue-no-flash", async ({
    authenticatedPage: page,
  }) => {
    // Hold the SECOND `has-patch` lookup (the one that fires after the user
    // adds i-seed00004 to the chip). See the matching comment in
    // issues-relation-filter.spec.ts for why we test "switch by add" rather
    // than "deselect, then select".
    let releaseSecondRelations: (() => void) | null = null;
    let combinedSourceCalls = 0;
    await page.route(/\/api\/v1\/relations(\?|$)/, async (route) => {
      const url = new URL(route.request().url());
      const sourceIds = url.searchParams.get("source_ids") ?? "";
      if (
        url.searchParams.get("rel_type") === "has-patch" &&
        sourceIds.includes("i-seed00002") &&
        sourceIds.includes("i-seed00004")
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

    await page.goto("/patches?relatedIssue=i-seed00002");
    await expect(page.getByTestId("filter-chip-relatedIssue")).toBeVisible();
    for (const id of I2_HAS_PATCHES) {
      await expect(page.getByTestId(`patches-list-row-${id}`)).toBeVisible();
    }

    // Open the chip's value picker.
    await page
      .getByTestId("filter-chip-relatedIssue")
      .getByRole("button", { name: /click to edit/i })
      .click();
    await expect(page.getByTestId("value-picker-relatedIssue")).toBeVisible();

    // Add i-seed00004 to the existing selection. The chip's values become
    // [i-seed00002, i-seed00004], which triggers a new relation lookup with
    // both source ids. Our route intercept holds that lookup mid-flight.
    await page.getByTestId("value-option-i-seed00004").click();

    await expect.poll(() => combinedSourceCalls).toBeGreaterThan(0);

    // While the resolver is held, the previous row(s) MUST persist. Multiple
    // observations in tight succession catch any transient empty paint.
    const rowSelector = page.locator(
      'tbody tr[data-testid^="patches-list-row-"]',
    );
    const observations: number[] = [];
    for (let i = 0; i < 6; i += 1) {
      observations.push(await rowSelector.count());
    }
    expect(Math.min(...observations)).toBe(I2_HAS_PATCHES.length);
    await expect(page.getByText("Loading patches…")).toHaveCount(0);
    await expect(
      page.getByText("No patches match the current filters."),
    ).toHaveCount(0);

    // Release the held call. The new union of i-seed00002 + i-seed00004
    // has-patch lands and the URL tracks the swap.
    expect(releaseSecondRelations).not.toBeNull();
    releaseSecondRelations?.();

    await expect(page).toHaveURL(
      /[?&]relatedIssue=i-seed00002(?:%2C|,)i-seed00004\b/,
    );
    for (const id of [...I2_HAS_PATCHES, ...I4_HAS_PATCHES]) {
      await expect(page.getByTestId(`patches-list-row-${id}`)).toBeVisible();
    }
  });
});
