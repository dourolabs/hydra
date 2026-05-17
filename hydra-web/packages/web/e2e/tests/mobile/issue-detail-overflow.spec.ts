import { test, expect } from "../../fixtures/auth";
import type { Page } from "@playwright/test";

// On mobile the sidebar drawer is open by default and intercepts pointer
// events on the page content underneath. Persist "hidden" before navigation
// so the drawer stays closed for assertions that need to interact with the
// main column. Mirrors the pattern in issue-detail.spec.ts.
async function setSidebarHidden(page: Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem("hydra-sidebar-hidden", "1");
  });
}

test.describe("Mobile Issue Detail overflow @mobile:issue-detail-overflow", () => {
  test.use({ viewport: { width: 375, height: 667 } });

  test("issue detail page does not overflow horizontally at 375px and SessionList is reachable @mobile:issue-detail-overflow", async ({
    authenticatedPage: page,
  }) => {
    await setSidebarHidden(page);
    // i-seed00002 has sessions ("t-seed00002") so the SessionList renders as a
    // table rather than the empty state — exercises the historically-overflowing
    // path.
    await page.goto("/issues/i-seed00002");

    // Wait for the heading so the page has fully laid out.
    await expect(
      page.getByRole("heading", { name: "Migrate authentication to OAuth2" }),
    ).toBeVisible();

    // Acceptance criterion 1: no horizontal overflow on the document.
    const documentOverflow = await page.evaluate(() => {
      const root = document.documentElement;
      return {
        scrollWidth: root.scrollWidth,
        clientWidth: root.clientWidth,
      };
    });
    expect(documentOverflow.scrollWidth).toBeLessThanOrEqual(documentOverflow.clientWidth + 1);

    // No in-page scroll container should have horizontal overflow either —
    // except deliberate horizontally-scrollable wrappers (e.g., the session
    // table wrapper, code blocks). Walk all elements and assert the page
    // root (.detail container's main pane) does not overflow horizontally.
    const mainOverflow = await page.evaluate(() => {
      // Find the IssueDetail .main pane via the SessionList container, then
      // walk up to the nearest scroll container.
      const list = document.querySelector('[data-testid="session-list"]');
      if (!list) return null;
      let node: HTMLElement | null = list.parentElement;
      while (node && node !== document.body) {
        const style = window.getComputedStyle(node);
        if (
          (style.overflowX === "auto" || style.overflowX === "scroll") &&
          node.getAttribute("data-testid") !== "session-list"
        ) {
          return {
            scrollWidth: node.scrollWidth,
            clientWidth: node.clientWidth,
            tag: node.tagName,
          };
        }
        node = node.parentElement;
      }
      return null;
    });
    if (mainOverflow) {
      expect(mainOverflow.scrollWidth).toBeLessThanOrEqual(mainOverflow.clientWidth + 1);
    }

    // Acceptance criterion 2: SessionList is reachable via vertical scroll.
    const sessionList = page.getByTestId("session-list");
    await sessionList.scrollIntoViewIfNeeded();
    await expect(sessionList).toBeVisible();
    await expect(sessionList).toBeInViewport();
  });
});
