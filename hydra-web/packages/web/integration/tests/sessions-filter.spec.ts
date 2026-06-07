import { test, expect } from "../fixtures/auth";

test.describe("Sessions list FilterBar @sessions:filter-bar", () => {
  test("auto-seeds creator, picks Status → running, then removes the creator chip @sessions:filter-bar", async ({
    authenticatedPage: page,
  }) => {
    const listSessionsRequests: { url: string }[] = [];
    page.on("request", (req) => {
      const u = req.url();
      // Match the list endpoint specifically (not /v1/sessions/<id>/...).
      if (
        u.match(/\/v1\/sessions(\?|$)/) ||
        u.match(/\/v1\/sessions\?/)
      ) {
        listSessionsRequests.push({ url: u });
      }
    });

    await page.goto("/sessions");
    await expect(page.getByTestId("sessions-list")).toBeVisible();

    // First-paint auto-seed: `creator=users/dev-user` is persisted to the URL
    // and the very first listSessions call narrows by the bare username.
    await expect(page).toHaveURL(/creator=users%2Fdev-user/);
    await expect
      .poll(() => listSessionsRequests.length, {
        message: "listSessions should be called at least once",
      })
      .toBeGreaterThan(0);
    expect(
      listSessionsRequests.some((r) => r.url.includes("creator=dev-user")),
    ).toBe(true);

    // The mock must honor the `creator` param server-side — assert the
    // rendered row set is narrowed, not just that the request URL carried
    // the param. With the seed fixture, `dev-user` owns a strict subset of
    // all sessions; capture the narrowed count for the post-removal compare.
    const rowLocator = page.locator('[data-testid^="sessions-list-row-"]');
    await expect(rowLocator.first()).toBeVisible();
    const creatorNarrowedCount = await rowLocator.count();
    expect(creatorNarrowedCount).toBeGreaterThan(0);

    // Open the + Filter menu and add a Status filter.
    await page.getByTestId("filter-bar-add").click();
    await expect(page.getByTestId("add-filter-menu")).toBeVisible();
    await page.getByTestId("add-filter-status").click();

    // The status value picker opens; pick "running".
    await expect(page.getByTestId("value-picker-status")).toBeVisible();
    await page.getByTestId("value-option-running").click();

    await expect(page).toHaveURL(/status=running/);
    // The new param triggers a re-fetch; confirm at least one listSessions
    // request now carries `status=running`.
    await expect
      .poll(() => {
        return listSessionsRequests.some((r) =>
          r.url.includes("status=running"),
        );
      })
      .toBe(true);

    // Close the picker.
    await page.keyboard.press("Escape");

    // Remove the auto-added creator chip. The chip exposes a remove button
    // labelled "Remove Creator filter".
    const creatorChip = page.getByTestId("filter-chip-creator");
    await expect(creatorChip).toBeVisible();
    await creatorChip
      .getByRole("button", { name: /remove creator filter/i })
      .click();

    // URL: creator stripped, status retained.
    await expect(page).not.toHaveURL(/creator=/);
    await expect(page).toHaveURL(/status=running/);

    // A subsequent listSessions call must be sent without a creator param.
    await expect
      .poll(() => {
        const after = listSessionsRequests.findLast((r) =>
          r.url.includes("status=running"),
        );
        return after !== undefined && !after.url.includes("creator=");
      })
      .toBe(true);

    // Row-set assertion: with the creator chip gone, the running-sessions row
    // set must grow beyond what `dev-user` alone owns. The seed has running
    // sessions from multiple creators; if the mock were ignoring `creator`,
    // this count would have matched `creatorNarrowedCount` from the start.
    await expect
      .poll(() => rowLocator.count())
      .toBeGreaterThan(creatorNarrowedCount);
  });
});
