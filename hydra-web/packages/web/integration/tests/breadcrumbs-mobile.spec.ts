import { test, expect } from "../fixtures/auth";

// The breadcrumb on an issue detail page renders three crumbs:
//   Workspace > Issues > <issueTitle>
// On mobile (≤768px) only the trailing (current) crumb should be visible;
// on desktop the full trail must stay intact.

test.describe("Breadcrumbs mobile collapse @mobile:breadcrumbs", () => {
  test("collapses to only the current crumb at 375px @mobile:breadcrumbs", async ({
    authenticatedPage: page,
  }) => {
    await page.setViewportSize({ width: 375, height: 667 });
    await page.goto("/issues/i-seed00001");

    const breadcrumb = page.locator('nav[aria-label="Breadcrumb"]');
    await expect(breadcrumb).toBeVisible();

    // The current crumb (the issue title) must be visible.
    await expect(
      breadcrumb.getByText("Platform v2.0 Migration"),
    ).toBeVisible();

    // Ancestor links are present in the DOM but collapsed via display:none.
    await expect(breadcrumb.getByRole("link", { name: "Workspace" })).toBeHidden();
    await expect(breadcrumb.getByRole("link", { name: "Issues" })).toBeHidden();

    // No layout space is consumed by the hidden ancestors — the breadcrumb
    // sits on a single line. We approximate "single line" by asserting the
    // height is well under what two stacked rows would occupy.
    const box = await breadcrumb.boundingBox();
    expect(box).not.toBeNull();
    expect(box!.height).toBeLessThan(40);
  });

  test("shows the full crumb trail at 1280px @mobile:breadcrumbs", async ({
    authenticatedPage: page,
  }) => {
    await page.setViewportSize({ width: 1280, height: 800 });
    await page.goto("/issues/i-seed00001");

    const breadcrumb = page.locator('nav[aria-label="Breadcrumb"]');
    await expect(breadcrumb).toBeVisible();

    await expect(breadcrumb.getByRole("link", { name: "Workspace" })).toBeVisible();
    await expect(breadcrumb.getByRole("link", { name: "Issues" })).toBeVisible();
    await expect(
      breadcrumb.getByText("Platform v2.0 Migration"),
    ).toBeVisible();
  });
});
