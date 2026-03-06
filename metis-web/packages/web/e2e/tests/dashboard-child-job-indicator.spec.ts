import { test, expect } from "../fixtures/auth";

test.describe("Dashboard Child Job Indicator @dashboard:child-job-indicator", () => {
  test("shows pulsing status box for child issue with running job @dashboard:child-job-indicator", async ({
    authenticatedPage: page,
  }) => {
    // Navigate to dashboard with everything filter
    await page.goto("/?selected=everything");

    // Find the row for "Platform v2.0 Migration" (i-seed00001)
    // which has child i-seed00002 with a running job t-seed00002
    const row = page.getByRole("button", {
      name: /Platform v2\.0 Migration/,
    });
    await expect(row).toBeVisible();

    // Verify the row's status dot is pulsing (hasRunningJob causes statusDotPulsing class)
    const statusDot = row.locator("span").first();
    const statusDotClass = await statusDot.getAttribute("class");
    expect(statusDotClass).toContain("statusDotPulsing");

    // Verify StatusBoxes are rendered (child status indicators)
    // StatusBoxes are 7x7px spans inside the rightColumn
    const statusBoxes = row.locator("span[class*='statusBox']");
    await expect(statusBoxes.first()).toBeVisible();

    // Verify at least one status box has the active/pulsing class
    // (the child i-seed00002 has a running job, so its box should be statusBoxActive)
    const activeBox = row.locator("span[class*='statusBoxActive']");
    await expect(activeBox).toHaveCount(1);

    // Verify the active box has a pulse animation
    const animationName = await activeBox.evaluate(
      (el) => getComputedStyle(el).animationName,
    );
    expect(animationName).not.toBe("none");
  });
});
