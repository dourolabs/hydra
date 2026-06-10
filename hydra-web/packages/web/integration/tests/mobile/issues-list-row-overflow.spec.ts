import { test, expect } from "../../fixtures/auth";
import type { Page } from "@playwright/test";

// On mobile the sidebar drawer is open by default and intercepts pointer
// events on the page content underneath. Persist "hidden" before the
// navigation that measures overflow so the drawer stays closed (and out of
// the layout flow) before assertions run. Mirrors the pattern in
// list-pages-overflow.spec.ts.
async function setSidebarHidden(page: Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem("hydra-sidebar-hidden", "1");
  });
}

const AUTH_HEADER = { Authorization: "Bearer dev-token-12345" };

// Seed an issue with content that would push the IssueRailRow past the
// viewport if any of its title / meta children allowed intrinsic overflow:
//
//   * Title is a single unbreakable token wider than the viewport.
//   * Status label (resolved.label) is a long phrase that would otherwise
//     `white-space: nowrap; flex-shrink: 0` itself past the meta wrap.
//   * Assignee is an agent whose display name doesn't fit on one line; the
//     Avatar still has a fixed size but the row title attribute carries the
//     full string and the principal display name is consumed by other rail
//     branches.
async function seedLongRowContent() {
  // Create issues whose content would push the IssueRailRow past a narrow
  // mobile viewport if the row didn't clip with `overflow: hidden`: a single
  // unbreakable title token, and a hyphenated identifier longer than any
  // narrow breakpoint.
  for (const title of [
    "Unbreakablesuperlongidentifierwithoutanywordboundariesthatmustnotpushtherowwidth",
    "a-really-long-hyphenated-identifier-that-cant-be-broken-at-natural-word-boundaries-and-must-be-truncated",
  ]) {
    const res = await fetch("http://localhost:8080/v1/issues", {
      method: "POST",
      headers: { "Content-Type": "application/json", ...AUTH_HEADER },
      body: JSON.stringify({
        issue: {
          title,
          description: "regression: mobile row truncation",
          type: "bug",
          status: "open",
          creator: "jayantk",
          assignee: {
            Agent: { name: "swe-with-an-unusually-long-agent-handle-for-mobile" },
          },
          project_id: "j-defaul",
        },
      }),
    });
    if (!res.ok) {
      throw new Error(`seed long-row issue failed: ${res.status} ${await res.text()}`);
    }
  }
}

const VIEWPORT_WIDTHS = [320, 360, 375] as const;

for (const viewportWidth of VIEWPORT_WIDTHS) {
  test.describe(`Mobile issues list row overflow @ ${viewportWidth}px @mobile:issues-row-overflow`, () => {
    test.use({ viewport: { width: viewportWidth, height: 667 } });

    test(`issue rows truncate cleanly with no horizontal overflow at ${viewportWidth}px @mobile:issues-row-overflow`, async ({
      authenticatedPage: page,
    }) => {
      await seedLongRowContent();
      await setSidebarHidden(page);
      await page.goto("/?selected=all");

      await expect(page.getByRole("heading", { name: "All issues", level: 1 })).toBeVisible();
      await page.waitForLoadState("networkidle");

      // Document and <main> must not surface a horizontal scrollbar.
      const documentOverflow = await page.evaluate(() => {
        const root = document.documentElement;
        return { scrollWidth: root.scrollWidth, clientWidth: root.clientWidth };
      });
      expect(
        documentOverflow.scrollWidth,
        `@${viewportWidth} doc scrollWidth=${documentOverflow.scrollWidth} clientWidth=${documentOverflow.clientWidth}`,
      ).toBeLessThanOrEqual(documentOverflow.clientWidth + 1);

      const mainOverflow = await page.evaluate(() => {
        const main = document.querySelector("main");
        return main ? { scrollWidth: main.scrollWidth, clientWidth: main.clientWidth } : null;
      });
      expect(mainOverflow, `@${viewportWidth} missing <main>`).not.toBeNull();
      expect(
        mainOverflow!.scrollWidth,
        `@${viewportWidth} main scrollWidth=${mainOverflow!.scrollWidth} clientWidth=${mainOverflow!.clientWidth}`,
      ).toBeLessThanOrEqual(mainOverflow!.clientWidth + 1);

      // Every visible IssueRailRow must fit inside the viewport. This catches
      // the case where a single row's intrinsic width exceeds the viewport
      // but its parent's overflow:auto masks the document-level scrollbar
      // (we still don't want a per-row scroller surfacing).
      const rowOverflows = await page.evaluate((vw: number) => {
        const rows = Array.from(
          document.querySelectorAll('[data-testid^="related-rail-row-issue-"]'),
        );
        const offenders: { id: string | undefined; w: number; right: number }[] = [];
        for (const row of rows) {
          const r = row.getBoundingClientRect();
          if (r.width > vw + 1 || r.right > vw + 1) {
            offenders.push({
              id: (row as HTMLElement).dataset?.testid,
              w: Math.round(r.width),
              right: Math.round(r.right),
            });
          }
        }
        return offenders;
      }, viewportWidth);
      expect(
        rowOverflows,
        `@${viewportWidth} rows extending past viewport: ${JSON.stringify(rowOverflows, null, 2)}`,
      ).toEqual([]);
    });
  });
}
