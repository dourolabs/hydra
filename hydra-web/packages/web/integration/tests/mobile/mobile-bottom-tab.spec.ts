import { test, expect } from "../../fixtures/auth";
import type { Page } from "@playwright/test";

// MobileBottomTabBar — persistent bottom-nav bar that replaces the
// hamburger-only cross-section navigation on mobile (≤768px).
//
// AppLayout decides whether to mount the bar based on `useMediaQuery("(max-width: 768px)")`,
// so the contract under test is "what the user sees at the configured viewport".
// At ≤768px the bar must be visible at the bottom of the viewport on every route
// inside AppLayout; at >768px the component must not be rendered at all.

const PRIMARY_TABS = [
  { id: "issues", path: "/", label: "Issues" },
  { id: "patches", path: "/patches", label: "Patches" },
  { id: "sessions", path: "/sessions", label: "Sessions" },
  { id: "chat", path: "/chat", label: "Chat" },
] as const;

// On mobile the sidebar drawer defaults to visible and slides in, covering
// the rest of the page including the tab bar. Persist hidden=true so the
// drawer stays closed throughout the tests that exercise the bar itself.
async function setSidebarHidden(page: Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem("hydra-sidebar-hidden", "1");
  });
}

test.describe("Mobile bottom-tab bar @mobile:bottom-tab", () => {
  test.describe("at 375x812 viewport", () => {
    test.use({ viewport: { width: 375, height: 812 } });

    test("bar is visible on every primary route and lights the matching tab @mobile:bottom-tab", async ({
      authenticatedPage: page,
    }) => {
      await setSidebarHidden(page);
      await page.goto("/");

      const bar = page.getByTestId("mobile-bottom-tab-bar");
      await expect(bar).toBeVisible();

      for (const tab of PRIMARY_TABS) {
        await page.goto(tab.path);
        await expect(bar).toBeVisible();
        const tabEl = page.getByTestId(`mobile-bottom-tab-${tab.id}`);
        await expect(tabEl).toHaveAttribute("data-active", "true");
        await expect(tabEl).toHaveAttribute("aria-current", "page");
      }
    });

    test("tapping a primary tab navigates and updates the active tab @mobile:bottom-tab", async ({
      authenticatedPage: page,
    }) => {
      await setSidebarHidden(page);
      await page.goto("/");
      await expect(page.getByTestId("mobile-bottom-tab-bar")).toBeVisible();

      // Use the bar to navigate to Patches.
      await page.getByTestId("mobile-bottom-tab-patches").click();
      await page.waitForURL((url) => url.pathname === "/patches");
      await expect(page.getByTestId("mobile-bottom-tab-patches")).toHaveAttribute(
        "data-active",
        "true",
      );
      await expect(page.getByTestId("mobile-bottom-tab-issues")).not.toHaveAttribute(
        "data-active",
        "true",
      );

      // And then to Sessions.
      await page.getByTestId("mobile-bottom-tab-sessions").click();
      await page.waitForURL((url) => url.pathname === "/sessions");
      await expect(page.getByTestId("mobile-bottom-tab-sessions")).toHaveAttribute(
        "data-active",
        "true",
      );
    });

    test("routes outside the primary four highlight the More tab @mobile:bottom-tab", async ({
      authenticatedPage: page,
    }) => {
      await setSidebarHidden(page);
      for (const path of ["/agents", "/secrets", "/repositories", "/projects"]) {
        await page.goto(path);
        const more = page.getByTestId("mobile-bottom-tab-more");
        await expect(more).toBeVisible();
        await expect(more).toHaveAttribute("data-active", "true");
      }
    });

    test("More opens the sidebar drawer @mobile:bottom-tab", async ({
      authenticatedPage: page,
    }) => {
      // Start with the sidebar collapsed so the drawer is closed.
      await setSidebarHidden(page);
      await page.goto("/");

      // Drawer hidden initially — the layout's data-sidebar attribute reflects
      // the current mode and the backdrop is absent.
      const layout = page.locator('[data-sidebar]');
      await expect(layout).toHaveAttribute("data-sidebar", "hidden");
      await expect(page.getByTestId("sidebar-backdrop")).toHaveCount(0);

      await page.getByTestId("mobile-bottom-tab-more").click();

      // Drawer is now open: data-sidebar flips to "open" and the backdrop appears.
      await expect(layout).toHaveAttribute("data-sidebar", "open");
      await expect(page.getByTestId("sidebar-backdrop")).toBeVisible();
    });

    test("bar sits flush with the viewport bottom and reserves room above the safe-area inset @mobile:bottom-tab", async ({
      authenticatedPage: page,
    }) => {
      await setSidebarHidden(page);
      await page.goto("/");
      const bar = page.getByTestId("mobile-bottom-tab-bar");
      await expect(bar).toBeVisible();

      const viewport = page.viewportSize();
      if (!viewport) throw new Error("viewport size not available");
      const box = await bar.boundingBox();
      if (!box) throw new Error("bar bounding box not available");
      // The bar's bottom edge should be at the viewport bottom (allowing a 1px
      // rounding tolerance), and its top edge should be at least
      // var(--mobile-nav-height) (56px) above the bottom.
      expect(box.y + box.height).toBeGreaterThanOrEqual(viewport.height - 1);
      expect(viewport.height - box.y).toBeGreaterThanOrEqual(56);

      // Inject a simulated safe-area inset and assert the bar grows to keep
      // the touch targets above it.
      await page.addStyleTag({
        content: `:root { --safe-area-bottom: 34px !important; }`,
      });
      const grownBox = await bar.boundingBox();
      if (!grownBox) throw new Error("bar bounding box (post-inset) not available");
      expect(grownBox.height).toBeGreaterThanOrEqual(56 + 34 - 1);
    });

    test("main scroll container reserves room so content does not sit under the bar @mobile:bottom-tab", async ({
      authenticatedPage: page,
    }) => {
      await setSidebarHidden(page);
      await page.goto("/");
      // Wait for the layout to render before reading the main element.
      await expect(page.getByTestId("mobile-bottom-tab-bar")).toBeVisible();

      // padding-bottom on the AppLayout <main> must be at least mobile-nav-height
      // so any sticky-bottom child (e.g. the chat composer) renders above the bar.
      const paddingBottom = await page.evaluate(() => {
        const main = document.querySelector("main");
        if (!main) throw new Error("AppLayout <main> not found");
        return parseFloat(window.getComputedStyle(main).paddingBottom);
      });
      expect(paddingBottom).toBeGreaterThanOrEqual(56);
    });
  });

  test.describe("at 1280x800 viewport", () => {
    test.use({ viewport: { width: 1280, height: 800 } });

    test("the bar is not rendered on desktop @mobile:bottom-tab", async ({
      authenticatedPage: page,
    }) => {
      await setSidebarHidden(page);
      await page.goto("/");
      await expect(page.getByTestId("mobile-bottom-tab-bar")).toHaveCount(0);
    });
  });
});
