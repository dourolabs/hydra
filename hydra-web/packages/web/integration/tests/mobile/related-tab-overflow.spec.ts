import { test, expect } from "../../fixtures/auth";
import type { Page } from "@playwright/test";

// On mobile the sidebar drawer is open by default and intercepts pointer
// events on the page content underneath. Persist "hidden" before navigation
// so the drawer stays closed for assertions that need to interact with the
// main column. Mirrors the pattern in list-pages-overflow.spec.ts and
// issue-detail-overflow.spec.ts.
async function setSidebarHidden(page: Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem("hydra-sidebar-hidden", "1");
  });
}

interface TabCase {
  /** Test-id of the mobile tab bar button to activate before measuring. */
  tabTestId: string;
  /** Human-readable label used in the test title. */
  label: string;
}

interface DetailCase {
  /** URL path to visit before clicking the tab. */
  path: string;
  /** Heading text rendered once the page has laid out — readiness signal. */
  heading: string;
  /** Mobile-tab-bar buttons to activate, in turn. */
  tabs: TabCase[];
}

/** Three narrow mobile breakpoints — small Android (360), iPhone SE/Mini
 *  (375), iPhone Pro/Plus (390–414, sampled here at 400). The 360 column
 *  catches offenders that escape the historical 375-only assertion (e.g. the
 *  four-tab MobileTabBar that overflows at 360). */
const VIEWPORT_WIDTHS = [360, 375, 400] as const;

const DETAIL_CASES: DetailCase[] = [
  {
    // i-seed00002 has sessions, so the Activity tab populates with timeline
    // entries and the Related tab can surface real rail rows. This exercises
    // the historically-overflowing four-tab MobileTabBar.
    path: "/issues/i-seed00002",
    // The detail H1 was removed in favor of the breadcrumb title — no
    // page-heading readiness signal is available for the issue detail case.
    heading: "",
    tabs: [
      { tabTestId: "issue-mobile-tab-related", label: "Related" },
      { tabTestId: "issue-mobile-tab-activity", label: "Activity" },
      { tabTestId: "issue-mobile-tab-details", label: "Details" },
    ],
  },
  {
    // c-seed00001 renders the chat-detail three-tab bar.
    path: "/chat/c-seed00001",
    heading: "",
    tabs: [
      { tabTestId: "chat-mobile-tab-related", label: "Related" },
      { tabTestId: "chat-mobile-tab-details", label: "Details" },
    ],
  },
];

async function assertNoOverflow(page: Page, viewportWidth: number, label: string) {
  // 1. Document scrollWidth.
  const documentOverflow = await page.evaluate(() => {
    const root = document.documentElement;
    return { scrollWidth: root.scrollWidth, clientWidth: root.clientWidth };
  });
  expect(
    documentOverflow.scrollWidth,
    `${label} @${viewportWidth} doc scrollWidth=${documentOverflow.scrollWidth} clientWidth=${documentOverflow.clientWidth}`,
  ).toBeLessThanOrEqual(documentOverflow.clientWidth + 1);

  // 2. AppLayout's <main> scrollWidth — page-level overflow contained inside
  //    main without bubbling to documentElement.
  const mainOverflow = await page.evaluate(() => {
    const main = document.querySelector("main");
    if (!main) return null;
    return { scrollWidth: main.scrollWidth, clientWidth: main.clientWidth };
  });
  expect(mainOverflow, `${label} @${viewportWidth} missing <main>`).not.toBeNull();
  expect(
    mainOverflow!.scrollWidth,
    `${label} @${viewportWidth} main scrollWidth=${mainOverflow!.scrollWidth} clientWidth=${mainOverflow!.clientWidth}`,
  ).toBeLessThanOrEqual(mainOverflow!.clientWidth + 1);

  // 3. Bounding-box walk — catches elements that extend past the viewport
  //    even when a parent's overflow clips them visually. Skip the off-screen
  //    sidebar drawer and any descendant of a deliberate `overflow-x:
  //    auto|scroll` scroll container (e.g. SessionList's tableWrapper),
  //    matching the project's mobile-test convention.
  const offenders = await page.evaluate((vw: number) => {
    const list: { sel: string; right: number; w: number }[] = [];
    const drawer = document.querySelector('[class*="_sidebarSlot_"]');
    const scrollers: Element[] = [];
    for (const el of document.querySelectorAll("*")) {
      const s = window.getComputedStyle(el);
      if (s.overflowX === "auto" || s.overflowX === "scroll") scrollers.push(el);
    }
    const all = document.querySelectorAll("*");
    for (const el of all) {
      if (drawer && (el === drawer || drawer.contains(el))) continue;
      let inScroller = false;
      for (const sc of scrollers) {
        if (sc === el) continue;
        if (sc.contains(el)) {
          inScroller = true;
          break;
        }
      }
      if (inScroller) continue;
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
  }, viewportWidth);
  expect(
    offenders,
    `${label} @${viewportWidth} elements extending past viewport: ${JSON.stringify(offenders, null, 2)}`,
  ).toEqual([]);

  // 4. Per-rail-row content fit. The Related tab's chat-panel `aside` is an
  //    `overflow-x: auto` container, so a row whose content overflows would
  //    scroll *inside* the panel without bubbling to <main> or
  //    documentElement — the previous three assertions all stay green. Walk
  //    each rendered rail row and require its content fit its own width, so a
  //    regression like long document paths missing the truncate pattern is
  //    caught before users see an inner scrollbar on the Related pane.
  const rowOverflow = await page.evaluate(() => {
    const rows = document.querySelectorAll('[data-testid^="related-rail-row-"]');
    const offenders: { testid: string; scrollWidth: number; clientWidth: number }[] = [];
    for (const row of rows) {
      const el = row as HTMLElement;
      if (el.scrollWidth > el.clientWidth + 1) {
        offenders.push({
          testid: el.dataset.testid ?? "",
          scrollWidth: el.scrollWidth,
          clientWidth: el.clientWidth,
        });
      }
    }
    return offenders;
  });
  expect(
    rowOverflow,
    `${label} @${viewportWidth} rail rows with internal overflow: ${JSON.stringify(rowOverflow, null, 2)}`,
  ).toEqual([]);
}

for (const viewportWidth of VIEWPORT_WIDTHS) {
  test.describe(`Mobile detail-tab overflow @ ${viewportWidth}px @mobile:related-tab-overflow`, () => {
    test.use({ viewport: { width: viewportWidth, height: 812 } });

    for (const { path, heading, tabs } of DETAIL_CASES) {
      test(`${path} tabs do not overflow horizontally at ${viewportWidth}px @mobile:related-tab-overflow`, async ({
        authenticatedPage: page,
      }) => {
        await setSidebarHidden(page);
        await page.goto(path);

        if (heading) {
          await expect(page.getByRole("heading", { name: heading }).first()).toBeVisible();
        }
        await page.waitForLoadState("networkidle");

        for (const { tabTestId, label } of tabs) {
          await page.getByTestId(tabTestId).click();
          // Wait for the tab pane to actually render before measuring —
          // `aria-selected` flips synchronously and the panel toggles via
          // display:none so there's no network round-trip to await.
          await expect(page.getByTestId(tabTestId)).toHaveAttribute("aria-selected", "true");
          await assertNoOverflow(page, viewportWidth, `${path}#${label}`);
        }
      });
    }
  });
}
