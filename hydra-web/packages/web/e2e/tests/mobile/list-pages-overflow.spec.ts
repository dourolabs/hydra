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

const VIEWPORT_WIDTH = 375;

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
  test.use({ viewport: { width: VIEWPORT_WIDTH, height: 667 } });

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

      // 1. The document itself must not overflow.
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

      // 2. AppLayout's <main> is `overflow: auto`, so a too-wide page-level
      //    layout creates an inner horizontal scrollbar without bubbling up
      //    to documentElement. Assert it directly. This is the gap that
      //    let the issues-toolbar Kbd hint slip past the original spec.
      const mainOverflow = await page.evaluate(() => {
        const main = document.querySelector("main");
        if (!main) return null;
        return { scrollWidth: main.scrollWidth, clientWidth: main.clientWidth };
      });
      expect(mainOverflow, `path=${path} missing <main>`).not.toBeNull();
      expect(
        mainOverflow!.scrollWidth,
        `path=${path} main scrollWidth=${mainOverflow!.scrollWidth} clientWidth=${mainOverflow!.clientWidth}`,
      ).toBeLessThanOrEqual(mainOverflow!.clientWidth + 1);

      // 3. Catch elements whose bounding box extends past the viewport even
      //    when their parent has overflow:hidden/auto/scroll masking it.
      //    Skip the sidebar drawer (off-screen by design when hidden).
      const offenders = await page.evaluate((vw: number) => {
        const list: { sel: string; right: number; w: number }[] = [];
        const drawer = document.querySelector('[class*="_sidebarSlot_"]');
        const all = document.querySelectorAll("*");
        for (const el of all) {
          if (drawer && (el === drawer || drawer.contains(el))) continue;
          const r = (el as HTMLElement).getBoundingClientRect();
          if (r.width === 0 && r.height === 0) continue;
          if (r.right > vw + 1) {
            const he = el as HTMLElement;
            const cls =
              typeof he.className === "string"
                ? "." + he.className.split(/\s+/).slice(0, 2).join(".")
                : "";
            list.push({
              sel: `${he.tagName.toLowerCase()}${cls}${he.dataset?.testid ? `[data-testid=${he.dataset.testid}]` : ""}`,
              right: r.right,
              w: r.width,
            });
          }
        }
        return list;
      }, VIEWPORT_WIDTH);
      expect(
        offenders,
        `path=${path} elements extending past viewport: ${JSON.stringify(offenders, null, 2)}`,
      ).toEqual([]);
    });
  }
});
