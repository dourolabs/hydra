import { test, expect } from "../../fixtures/auth";
import type { Page } from "@playwright/test";

// Verifies the AppLayout main scroll container reserves room for iOS Safari's
// home-indicator safe area on the issue detail page. See
// chat-bottom-safe-area.spec.ts for the rationale behind the CSS-variable
// override approach (Playwright's Chromium emulator does not inject a non-zero
// env(safe-area-inset-bottom)).

const SIMULATED_SAFE_AREA_PX = 34;
const BREATHING_ROOM_PX = 8;

async function setSidebarHidden(page: Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem("hydra-sidebar-hidden", "1");
  });
}

async function injectSimulatedSafeArea(page: Page, px: number) {
  await page.addStyleTag({
    content: `:root { --safe-area-bottom: ${px}px !important; }`,
  });
}

test.describe("Mobile issue detail bottom safe-area @mobile:issue-detail-bottom-safe-area", () => {
  test.use({ viewport: { width: 390, height: 844 } });

  test("AppLayout main reserves space for env(safe-area-inset-bottom) on issue detail @mobile:issue-detail-bottom-safe-area", async ({
    authenticatedPage: page,
  }) => {
    await setSidebarHidden(page);
    // i-seed00002 has sessions so SessionList renders as a table — matches the
    // existing @mobile:issue-detail-overflow fixture.
    await page.goto("/issues/i-seed00002");

    await expect(
      page.getByRole("heading", { name: "Migrate authentication to OAuth2" }),
    ).toBeVisible();

    // Hard stop: env(safe-area-inset-bottom) is gated behind viewport-fit=cover
    // on iOS Safari. Without the meta opt-in, the CSS fix is a no-op on the
    // platform that needs it most, regardless of how the calc reads in tests.
    const viewportMetaContent = await page
      .locator('meta[name="viewport"]')
      .getAttribute("content");
    expect(viewportMetaContent ?? "").toContain("viewport-fit=cover");

    await injectSimulatedSafeArea(page, SIMULATED_SAFE_AREA_PX);

    // The AppLayout `<main>` is the outermost scroll container whose
    // padding-bottom keeps the SessionList clear of the home indicator.
    const paddingBottom = await page.evaluate(() => {
      const main = document.querySelector("main");
      if (!main) throw new Error("AppLayout <main> not found");
      return parseFloat(window.getComputedStyle(main).paddingBottom);
    });

    expect(paddingBottom).toBeGreaterThanOrEqual(BREATHING_ROOM_PX + SIMULATED_SAFE_AREA_PX);

    // SessionList remains reachable via vertical scroll after the fix; scroll
    // the IssueDetail .main pane to its bottom and assert SessionList is in
    // the viewport. This guards against an over-aggressive padding push that
    // would shove the list past the visible area.
    const sessionList = page.getByTestId("session-list");
    await sessionList.scrollIntoViewIfNeeded();
    await expect(sessionList).toBeVisible();
    await expect(sessionList).toBeInViewport();

    // User-visible geometry: nothing in the inner issue-detail pane should
    // poke past the visible viewport. This guards against the regression where
    // a 100vh grid row pushes the inner scroller under the iOS toolbar.
    const issueMainBox = await page.getByTestId("issue-detail-main").boundingBox();
    const viewport = page.viewportSize();
    if (!issueMainBox) throw new Error("issue-detail-main bounding box not available");
    if (!viewport) throw new Error("viewport size not available");
    expect(issueMainBox.y + issueMainBox.height).toBeLessThanOrEqual(viewport.height);
  });
});
