import { test, expect } from "../fixtures/auth";

// Regression for i-jmmytc: at narrow viewports the main content collapsed to
// 0 width because .sidebarSlot (position: fixed on mobile) was removed from
// grid auto-placement, causing .contentColumn to fall into the 0-width first
// grid track instead of the 1fr track. We assert the <main> element renders
// with a non-zero box at every supported viewport width across the main
// authenticated pages.

const WIDTHS = [1440, 1024, 800, 768, 600, 500, 375] as const;

const PAGES = [
  { name: "issues", path: "/" },
  { name: "issues-all", path: "/?selected=all" },
  { name: "issue-detail", path: "/issues/i-seed00001" },
  { name: "chat", path: "/chat/c-seed00001" },
  { name: "document", path: "/documents/d-seed00001" },
  { name: "session-log", path: "/issues/i-seed00005/sessions/t-seed00001/logs" },
  { name: "patches", path: "/patches" },
] as const;

test.describe("Responsive layout — main content visibility @layout:responsive", () => {
  for (const { name, path } of PAGES) {
    for (const width of WIDTHS) {
      test(`${name} renders <main> at ${width}px @layout:responsive`, async ({
        authenticatedPage: page,
      }) => {
        // On mobile, the drawer is open by default. Pre-set localStorage to
        // hidden so the drawer is dismissed on load — this isolates the bug
        // (which manifested only once the drawer was closed) from any
        // legitimate "drawer overlay covers content" behavior.
        await page.addInitScript(() => {
          try {
            window.localStorage.setItem("hydra-sidebar-hidden", "1");
          } catch {
            /* ignore */
          }
        });

        await page.setViewportSize({ width, height: 800 });
        await page.goto(path);
        await page.waitForLoadState("domcontentloaded");
        await page.waitForTimeout(500);

        const main = page.locator("main");
        await expect(main).toBeVisible();

        const box = await main.boundingBox();
        expect(box, "<main> must have a bounding box").not.toBeNull();
        // The main column should fill the viewport (minus the sidebar on
        // desktop). At every supported width it must be visibly wide and tall.
        expect(box!.width, "<main> width").toBeGreaterThan(width * 0.5);
        expect(box!.height, "<main> height").toBeGreaterThan(200);

        // <main> must not be fully occluded by a higher z-index overlay. We
        // probe the center of the main element with elementFromPoint and
        // assert the resolved element is inside <main> (or <main> itself).
        const occluded = await page.evaluate(() => {
          const m = document.querySelector("main");
          if (!m) return "no-main";
          const r = m.getBoundingClientRect();
          const cx = r.left + r.width / 2;
          const cy = r.top + r.height / 2;
          const hit = document.elementFromPoint(cx, cy);
          if (!hit) return "no-hit";
          return m.contains(hit) ? null : hit.tagName + "." + hit.className;
        });
        expect(occluded, `<main> must not be occluded`).toBeNull();
      });
    }
  }
});

test.describe("Responsive layout — mobile drawer mechanics @layout:responsive-drawer", () => {
  test("mobile drawer can be opened via More tab and dismissed via backdrop @layout:responsive-drawer", async ({
    authenticatedPage: page,
  }) => {
    // Start in the dismissed state so the drawer must be explicitly opened.
    await page.addInitScript(() => {
      try {
        window.localStorage.setItem("hydra-sidebar-hidden", "1");
      } catch {
        /* ignore */
      }
    });
    await page.setViewportSize({ width: 375, height: 800 });
    await page.goto("/");
    await page.waitForLoadState("domcontentloaded");
    await page.waitForTimeout(300);

    // Drawer starts off-screen (transform: translateX(-100%)).
    const sidebar = page.locator('[class*="sidebarSlot"]').first();
    let box = await sidebar.boundingBox();
    expect(box).not.toBeNull();
    expect(box!.x).toBeLessThan(0);

    // The hamburger is hidden on mobile; the bottom-tab bar's "More" cell
    // is the canonical drawer entry point.
    await page.getByTestId("mobile-bottom-tab-more").click();
    await expect(page.getByTestId("sidebar-backdrop")).toBeVisible();
    await page.waitForTimeout(250);

    // Drawer is now on-screen.
    box = await sidebar.boundingBox();
    expect(box).not.toBeNull();
    expect(box!.x).toBeGreaterThanOrEqual(0);

    // Tap backdrop to dismiss. The drawer is z-index:30 above the backdrop
    // (z-index:20), so we click at the right edge where only the backdrop sits.
    const viewport = page.viewportSize()!;
    await page.mouse.click(viewport.width - 10, viewport.height / 2);
    await expect(page.getByTestId("sidebar-backdrop")).toBeHidden();
  });

  test("desktop sidebar collapses via hamburger and main remains visible @layout:responsive-drawer", async ({
    authenticatedPage: page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 800 });
    await page.goto("/");
    await page.waitForLoadState("domcontentloaded");
    await page.waitForTimeout(300);

    const main = page.locator("main");
    const sidebar = page.locator('[class*="sidebarSlot"]').first();

    const mainBefore = await main.boundingBox();
    const sidebarBefore = await sidebar.boundingBox();
    expect(mainBefore!.width).toBeGreaterThan(0);
    expect(sidebarBefore!.width).toBeGreaterThan(0);

    // Collapse the sidebar via the header hamburger toggle.
    await page.getByTestId("site-header-toggle-sidebar").click();
    await page.waitForTimeout(300);

    const sidebarAfter = await sidebar.boundingBox();
    const mainAfter = await main.boundingBox();
    expect(sidebarAfter!.width).toBe(0);
    expect(mainAfter!.width).toBeGreaterThan(mainBefore!.width);
  });
});
