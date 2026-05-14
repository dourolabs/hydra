import { test, expect } from "../fixtures/auth";

// site-header-sessions was dropped: the active-sessions slot is now a labelled
// pill ("no sessions" / "N sessions") with no wrapping Tooltip.
//
// The sidebar-hide control moved into the sidebar on desktop; the header
// hamburger (site-header-toggle-sidebar) only mounts on mobile or when the
// sidebar is already hidden. The desktop tooltip equivalent is now
// sidebar-hide inside the sidebar itself.
const VIEWPORTS = [
  {
    name: "desktop 1440x900",
    width: 1440,
    height: 900,
    triggers: ["sidebar-hide", "site-header-search"],
  },
  {
    name: "mobile 375x812",
    width: 375,
    height: 812,
    triggers: ["site-header-toggle-sidebar", "site-header-search"],
  },
];

test.describe("Header tooltips @nav:tooltip-viewport", () => {
  for (const vp of VIEWPORTS) {
    test(`stay within viewport at ${vp.name}`, async ({
      authenticatedPage: page,
    }) => {
      await page.setViewportSize({ width: vp.width, height: vp.height });
      await page.goto("/");

      for (const testid of vp.triggers) {
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
        // Move pointer away so the next tooltip can show cleanly.
        await page.mouse.move(0, vp.height - 1);
        await expect(tooltip).toBeHidden();
      }
    });
  }
});
