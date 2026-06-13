import { test, expect } from "../fixtures/auth";

// Only desktop matters here: the hamburger is the only Tooltip-wrapped
// trigger in SiteHeader, and the hamburger is hidden on mobile (replaced by
// the bottom-tab "More" cell).
const VIEWPORTS = [{ name: "desktop 1440x900", width: 1440, height: 900 }];

// Only triggers that are wrapped in a Tooltip in SiteHeader.tsx render a
// role="tooltip" on hover. Today that is the hamburger only — search,
// sessions, and the create menu are bare buttons.
const TOOLTIP_TRIGGERS = ["site-header-toggle-sidebar"];

test.describe("Header tooltips @nav:tooltip-viewport", () => {
  for (const vp of VIEWPORTS) {
    test(`stay within viewport at ${vp.name} @nav:tooltip-viewport`, async ({
      authenticatedPage: page,
    }) => {
      // Dismiss the mobile drawer so the hamburger isn't occluded by the
      // sidebar slot. On desktop this is a no-op (hidden state still shows
      // header + main, just with the sidebar collapsed).
      await page.addInitScript(() => {
        try {
          window.localStorage.setItem("hydra-sidebar-hidden", "1");
        } catch {
          /* ignore */
        }
      });
      await page.setViewportSize({ width: vp.width, height: vp.height });
      await page.goto("/");

      for (const testid of TOOLTIP_TRIGGERS) {
        const trigger = page.getByTestId(testid);
        await trigger.hover();

        const tooltip = page.getByRole("tooltip").first();
        await expect(tooltip).toBeVisible();

        const box = await tooltip.boundingBox();
        expect(
          box,
          `${testid}: tooltip should have a bounding box`,
        ).not.toBeNull();
        expect(
          box!.x,
          `${testid}: tooltip left within viewport`,
        ).toBeGreaterThanOrEqual(0);
        expect(
          box!.x + box!.width,
          `${testid}: tooltip right within viewport`,
        ).toBeLessThanOrEqual(vp.width);
        expect(
          box!.y,
          `${testid}: tooltip top within viewport`,
        ).toBeGreaterThanOrEqual(0);
        expect(
          box!.y + box!.height,
          `${testid}: tooltip bottom within viewport`,
        ).toBeLessThanOrEqual(vp.height);

        // Move pointer away so the next tooltip can show cleanly.
        await page.mouse.move(0, vp.height - 1);
        await expect(tooltip).toBeHidden();
      }
    });
  }
});
