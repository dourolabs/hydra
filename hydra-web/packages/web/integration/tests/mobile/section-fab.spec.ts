import { test, expect } from "../../fixtures/auth";
import type { Page } from "@playwright/test";

// Section FABs — section-aware "create new" Floating Action Button surfaced on
// mobile bottom-right (above the bottom-tab bar). The desktop SiteHeader
// create-menu is retired on mobile in favour of these per-section FABs.

async function setSidebarHidden(page: Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem("hydra-sidebar-hidden", "1");
  });
}

test.describe("Mobile section FABs @mobile:section-fab", () => {
  test.describe("at 375x812 viewport", () => {
    test.use({ viewport: { width: 375, height: 812 } });

    test("Issues list shows a FAB that opens the issue-create modal @mobile:section-fab", async ({
      authenticatedPage: page,
    }) => {
      await setSidebarHidden(page);
      await page.goto("/?selected=all");

      const fab = page.getByTestId("issues-fab");
      await expect(fab).toBeVisible();
      await expect(fab).toHaveAttribute("aria-label", "New issue");

      await fab.click();
      await expect(page.getByTestId("issue-create-modal")).toBeVisible();
    });

    test("Chat list shows a FAB that opens the chat-create modal @mobile:section-fab", async ({
      authenticatedPage: page,
    }) => {
      await setSidebarHidden(page);
      await page.goto("/chat");

      const fab = page.getByTestId("chats-fab");
      await expect(fab).toBeVisible();
      await expect(fab).toHaveAttribute("aria-label", "New chat");

      await fab.click();
      await expect(page.getByTestId("chat-create-modal")).toBeVisible();
    });

    test("no FAB on routes outside Issues / Chat @mobile:section-fab", async ({
      authenticatedPage: page,
    }) => {
      await setSidebarHidden(page);
      for (const path of ["/sessions", "/agents", "/patches"]) {
        await page.goto(path);
        await expect(page.getByTestId("issues-fab")).toHaveCount(0);
        await expect(page.getByTestId("chats-fab")).toHaveCount(0);
      }
    });

    test("FAB sits above the bottom-tab bar @mobile:section-fab", async ({
      authenticatedPage: page,
    }) => {
      await setSidebarHidden(page);
      await page.goto("/?selected=all");

      const fab = page.getByTestId("issues-fab");
      const bar = page.getByTestId("mobile-bottom-tab-bar");
      await expect(fab).toBeVisible();
      await expect(bar).toBeVisible();

      const fabBox = await fab.boundingBox();
      const barBox = await bar.boundingBox();
      if (!fabBox || !barBox) throw new Error("bounding box not available");

      // FAB's bottom edge must be above the bar's top edge.
      expect(fabBox.y + fabBox.height).toBeLessThanOrEqual(barBox.y);
      // And FAB sits roughly 16px to the right edge.
      const viewport = page.viewportSize();
      if (!viewport) throw new Error("viewport size not available");
      expect(viewport.width - (fabBox.x + fabBox.width)).toBeLessThanOrEqual(24);
    });

    test("FAB respects safe-area-inset-bottom @mobile:section-fab", async ({
      authenticatedPage: page,
    }) => {
      await setSidebarHidden(page);
      await page.goto("/?selected=all");
      const fab = page.getByTestId("issues-fab");
      await expect(fab).toBeVisible();
      const beforeBox = await fab.boundingBox();
      if (!beforeBox) throw new Error("bounding box (pre-inset) not available");

      await page.addStyleTag({
        content: `:root { --safe-area-bottom: 34px !important; }`,
      });
      const afterBox = await fab.boundingBox();
      if (!afterBox) throw new Error("bounding box (post-inset) not available");
      // The FAB shifts up by ~34px as the bar (and the FAB's bottom inset) grow.
      expect(beforeBox.y - afterBox.y).toBeGreaterThanOrEqual(30);
    });
  });

  test.describe("at 1280x800 viewport", () => {
    test.use({ viewport: { width: 1280, height: 800 } });

    test("the FAB is not rendered on desktop @mobile:section-fab", async ({
      authenticatedPage: page,
    }) => {
      await setSidebarHidden(page);
      await page.goto("/?selected=all");
      await expect(page.getByTestId("issues-fab")).toHaveCount(0);
      await page.goto("/chat");
      await expect(page.getByTestId("chats-fab")).toHaveCount(0);
    });
  });
});
