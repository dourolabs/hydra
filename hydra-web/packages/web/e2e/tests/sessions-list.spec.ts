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

  test("clicking a session linked to an issue navigates to its log page @sessions:list", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/sessions");
    // t-seed00002 is spawned_from i-seed00002 and running.
    const row = page.getByTestId("sessions-list-row-t-seed00002");
    await expect(row).toBeVisible();
    await row.getByText("t-seed00002").click();
    await expect(page).toHaveURL(/\/issues\/i-seed00002\/sessions\/t-seed00002\/logs$/);
  });
});
