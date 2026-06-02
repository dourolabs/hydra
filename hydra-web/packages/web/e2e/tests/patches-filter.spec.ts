import { test, expect } from "../fixtures/auth";

test.describe("Patches FilterBar @patches:filter-bar", () => {
  test("user can add a Status filter via the FilterBar; URL persists and reload re-renders the chip @patches:filter-bar", async ({
    authenticatedPage: page,
  }) => {
    // Capture every listPatches request so we can prove the chip drives a
    // server-side narrow (no client-side filtering of an unfiltered page).
    const listPatchesRequests: URL[] = [];
    page.on("request", (req) => {
      const url = new URL(req.url());
      if (url.pathname === "/api/v1/patches") {
        listPatchesRequests.push(url);
      }
    });

    await page.goto("/patches");
    await expect(page.getByRole("heading", { name: "Patches" })).toBeVisible();

    // The hand-rolled status-chip button row should be gone — replaced
    // wholesale by the generic FilterBar. The legacy `patches-filter-*`
    // testids no longer exist.
    await expect(page.getByTestId("patches-filter-all")).toHaveCount(0);
    await expect(page.getByTestId("filter-bar-add")).toBeVisible();

    // Open the add-filter menu and pick Status.
    await page.getByTestId("filter-bar-add").click();
    await expect(page.getByTestId("add-filter-menu")).toBeVisible();
    await page.getByTestId("add-filter-status").click();

    // The chip exists and the value picker is open.
    await expect(page.getByTestId("filter-chip-status")).toBeVisible();
    await expect(page.getByTestId("value-picker-status")).toBeVisible();

    // Pick Merged. URL reflects `?status=Merged`.
    await page.getByTestId("value-option-Merged").click();
    await expect(page).toHaveURL(/[?&]status=Merged\b/);

    // The server-side narrow fired with status=Merged.
    await page.waitForLoadState("networkidle");
    const narrowing = listPatchesRequests.filter(
      (u) => u.searchParams.get("status") === "Merged",
    );
    expect(narrowing.length).toBeGreaterThan(0);

    // A Merged-status patch is visible after the narrow.
    await expect(
      page.getByText("Implement OAuth2 scopes and permission mapping"),
    ).toBeVisible();

    // Reload preserves URL + chip — the page hydrates from `?status=Merged`.
    await page.reload();
    await expect(page).toHaveURL(/[?&]status=Merged\b/);
    await expect(page.getByTestId("filter-chip-status")).toBeVisible();
    await expect(
      page.getByText("Implement OAuth2 scopes and permission mapping"),
    ).toBeVisible();
  });
});
