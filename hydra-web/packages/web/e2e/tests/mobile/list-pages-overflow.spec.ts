import { test, expect } from "../../fixtures/auth";
import type { Page } from "@playwright/test";

// On mobile the sidebar drawer is open by default and intercepts pointer
// events on the page content underneath. Persist "hidden" before navigation
// so the drawer stays closed for assertions that need to interact with the
// main column. Mirrors the pattern in issue-detail-overflow.spec.ts.
async function setSidebarHidden(page: Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem("hydra-sidebar-hidden", "1");
  });
}

interface PageCase {
  path: string;
  /** Heading text rendered once the page has laid out. Every list page renders
   *  an <h1> with a stable title, so we use that as a readiness signal. */
  heading: string;
}

const PAGES: PageCase[] = [
  { path: "/sessions", heading: "Sessions" },
  { path: "/patches", heading: "Patches" },
  { path: "/", heading: "All issues" },
  { path: "/?selected=all", heading: "All issues" },
  { path: "/chat", heading: "Chats" },
  { path: "/repositories", heading: "Repositories" },
  { path: "/agents", heading: "Agents" },
  { path: "/secrets", heading: "Secrets" },
];

test.describe("Mobile list pages overflow @mobile:list-overflow", () => {
  test.use({ viewport: { width: 375, height: 667 } });

  for (const { path, heading } of PAGES) {
    test(`list page ${path} does not overflow horizontally at 375px @mobile:list-overflow`, async ({
      authenticatedPage: page,
    }) => {
      await setSidebarHidden(page);
      await page.goto(path);

      // Wait for the heading so the page has fully laid out, then for the
      // network to settle so list rows have rendered.
      await expect(page.getByRole("heading", { name: heading, level: 1 })).toBeVisible();
      await page.waitForLoadState("networkidle");

      const documentOverflow = await page.evaluate(() => {
        const root = document.documentElement;
        return {
          scrollWidth: root.scrollWidth,
          clientWidth: root.clientWidth,
        };
      });
      expect(
        documentOverflow.scrollWidth,
        `path=${path} scrollWidth=${documentOverflow.scrollWidth} clientWidth=${documentOverflow.clientWidth}`,
      ).toBeLessThanOrEqual(documentOverflow.clientWidth + 1);
    });
  }
});
