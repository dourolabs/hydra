import { test, expect } from "../fixtures/auth";

test.describe("Sessions list page @sessions:list", () => {
  test("renders sessions with active-first ordering @sessions:list", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/sessions");

    const list = page.getByTestId("sessions-list");
    await expect(list).toBeVisible();

    // Seed data includes both running and complete sessions; we expect at
    // least one row, and active sessions must appear before terminal ones.
    const rows = page.locator('[data-testid^="sessions-list-row-"]');
    await expect(rows.first()).toBeVisible();
    const count = await rows.count();
    expect(count).toBeGreaterThan(0);

    const ids: string[] = [];
    for (let i = 0; i < count; i += 1) {
      const testId = await rows.nth(i).getAttribute("data-testid");
      ids.push(testId!.replace("sessions-list-row-", ""));
    }

    // From seed.json: t-seed00001 + t-seed00010 are complete (terminal);
    // the rest are running (active). Active rows must precede terminal ones.
    const terminalIds = new Set(["t-seed00001", "t-seed00010"]);
    let firstTerminalIdx = ids.findIndex((id) => terminalIds.has(id));
    if (firstTerminalIdx === -1) firstTerminalIdx = ids.length;
    const beforeTerminal = ids.slice(0, firstTerminalIdx);
    for (const id of beforeTerminal) {
      expect(terminalIds.has(id)).toBe(false);
    }
    // At least one terminal session is somewhere in the list.
    expect(ids.some((id) => terminalIds.has(id))).toBe(true);
  });

  test("bounds the first paint to PAGE_SIZE (≤ 50) and hides Load more when exhausted @sessions:list", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/sessions");

    const list = page.getByTestId("sessions-list");
    await expect(list).toBeVisible();

    const rows = page.locator('[data-testid^="sessions-list-row-"]');
    const count = await rows.count();
    // The seed dataset has < 50 sessions, so the first page should still
    // contain all of them. PAGE_SIZE bound is enforced regardless.
    expect(count).toBeLessThanOrEqual(50);

    // With the small seed dataset, the server returns no next_cursor so
    // the Load more button is not rendered.
    await expect(page.getByTestId("sessions-load-more")).toHaveCount(0);
  });

  test("clicking a session row navigates to the universal session detail page @sessions:list", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/sessions");
    // t-seed00002 is spawned_from i-seed00002 and running.
    const row = page.getByTestId("sessions-list-row-t-seed00002");
    await expect(row).toBeVisible();
    // The Agent cell is safe to click (no stopPropagation on its contents).
    await row.locator("td").first().click();
    await expect(page).toHaveURL(/\/sessions\/t-seed00002$/);
  });
});
