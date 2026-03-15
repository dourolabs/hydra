import { test, expect } from "../fixtures/auth";

test.describe("Dashboard Child Session Indicator @dashboard:child-session-indicator", () => {
  test("shows pulsing status box for child issue with running session @dashboard:child-session-indicator", async ({
    authenticatedPage: page,
  }) => {
    // Navigate to dashboard with everything filter
    await page.goto("/?selected=everything");

    // Find the row for "Platform v2.0 Migration" (i-seed00001)
    // which has child i-seed00002 with a running session t-seed00002
    const row = page.getByRole("button", {
      name: /Platform v2\.0 Migration/,
    });
    await expect(row).toBeVisible();

    // Verify the row's status dot is pulsing (hasRunningSession causes statusDotPulsing class)
    const statusDot = row.locator("span").first();
    await expect(statusDot).toHaveClass(/statusDotPulsing/);

    // Verify StatusBoxes are rendered (child status indicators)
    // StatusBoxes are 7x7px spans inside the rightColumn
    const statusBoxes = row.locator("span[class*='statusBox']");
    await expect(statusBoxes.first()).toBeVisible();

    // Verify at least one status box has the active/pulsing class
    // (children with running sessions should show statusBoxActive)
    const activeBox = row.locator("span[class*='statusBoxActive']");
    const activeCount = await activeBox.count();
    expect(activeCount).toBeGreaterThanOrEqual(1);

    // Verify the first active box has a pulse animation
    const animationName = await activeBox.first().evaluate(
      (el) => getComputedStyle(el).animationName,
    );
    expect(animationName).not.toBe("none");
  });
});
