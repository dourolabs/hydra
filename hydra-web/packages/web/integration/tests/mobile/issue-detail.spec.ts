import { test, expect } from "../../fixtures/auth";
import type { Page } from "@playwright/test";

// On mobile the sidebar drawer is open by default and intercepts pointer
// events on the page content underneath. Persist "hidden" before navigation
// so the drawer stays closed for assertions that need to interact with the
// main column. Mirrors the pattern in chat-tabs.spec.ts.
async function setSidebarHidden(page: Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem("hydra-sidebar-hidden", "1");
  });
}

test.describe("Mobile Issue Detail @mobile:issue-detail", () => {
  test.describe("at 375x700 viewport", () => {
    test.use({ viewport: { width: 375, height: 700 } });

    test("displays issue title in breadcrumb and id chip @mobile:issue-detail", async ({
      authenticatedPage: page,
    }) => {
      await setSidebarHidden(page);
      await page.goto("/issues/i-seed00002");

      // Breadcrumb's trailing crumb is the issue title (on mobile the trail
      // collapses to just the current crumb).
      const breadcrumb = page.locator('nav[aria-label="Breadcrumb"]');
      await expect(breadcrumb).toBeVisible();
      await expect(
        breadcrumb.getByText("Migrate authentication to OAuth2"),
      ).toBeVisible();

      // The id chip stays visible in the detail body's title row.
      const idChip = page
        .getByRole("main")
        .getByText("i-seed00002", { exact: true });
      await expect(idChip).toBeVisible();
      await expect(idChip).toBeInViewport();

      // Each pane owns its own scroll — the page body must not have scrolled.
      const docScroll = await page.evaluate(
        () => document.scrollingElement?.scrollTop ?? window.scrollY,
      );
      expect(docScroll).toBe(0);
    });

    test("top tabs toggle between Overview, Related, Activity, Details @mobile:issue-detail", async ({
      authenticatedPage: page,
    }) => {
      await setSidebarHidden(page);
      await page.goto("/issues/i-seed00001");

      const overviewTab = page.getByTestId("issue-mobile-tab-overview");
      const relatedTab = page.getByTestId("issue-mobile-tab-related");
      const activityTab = page.getByTestId("issue-mobile-tab-activity");
      const detailsTab = page.getByTestId("issue-mobile-tab-details");

      // All four tabs should be visible in the mobile tab bar.
      await expect(overviewTab).toBeVisible();
      await expect(relatedTab).toBeVisible();
      await expect(activityTab).toBeVisible();
      await expect(detailsTab).toBeVisible();
      await expect(overviewTab).toHaveAttribute("aria-selected", "true");

      // Overview pane shows the id chip in the title row (the heading was
      // removed in favor of the breadcrumb title).
      const idChip = page.getByRole("main").getByText("i-seed00001", { exact: true });
      await expect(idChip).toBeVisible();

      // The status chip lives in the rail (Details tab) — hidden on Overview.
      const statusChip = page.getByTestId("status-chip");
      await expect(statusChip).toBeHidden();

      // Switch to Related — section headings (or empty states) appear.
      await relatedTab.click();
      await expect(relatedTab).toHaveAttribute("aria-selected", "true");
      await expect(overviewTab).toHaveAttribute("aria-selected", "false");
      await expect(idChip).toBeHidden();
      await expect(
        page.getByRole("heading", { name: /Parents|Children|Patches|Documents/ }).first(),
      ).toBeVisible();

      // Switch to Activity — activity timeline should surface.
      await activityTab.click();
      await expect(activityTab).toHaveAttribute("aria-selected", "true");
      // The activity timeline always renders an "Issue created" creation entry
      // for any issue with versions; assert on the underlying creation label.
      await expect(page.getByText("Issue created").first()).toBeVisible();

      // Switch to Details — rail content (status chip, Created label) visible.
      // Scope to the rail (complementary landmark) since SessionList also
      // renders a "Created" column header.
      await detailsTab.click();
      await expect(detailsTab).toHaveAttribute("aria-selected", "true");
      await expect(statusChip).toBeVisible();
      await expect(
        page.getByRole("complementary").getByText("Created", { exact: true }),
      ).toBeVisible();

      // Switch back to Overview — id chip is back, rail hidden.
      await overviewTab.click();
      await expect(overviewTab).toHaveAttribute("aria-selected", "true");
      await expect(idChip).toBeVisible();
      await expect(statusChip).toBeHidden();
    });

    test("status chip in Details opens update modal @mobile:issue-detail", async ({
      authenticatedPage: page,
    }) => {
      await setSidebarHidden(page);
      await page.goto("/issues/i-seed00005");

      // Status chip lives in the rail — must switch to Details first.
      await page.getByTestId("issue-mobile-tab-details").click();
      const statusChip = page.getByTestId("status-chip");
      await expect(statusChip).toBeVisible();

      await statusChip.click();
      const modal = page.getByRole("dialog");
      await expect(modal).toBeVisible();
    });
  });

  test.describe("at 1280x720 viewport", () => {
    test.use({ viewport: { width: 1280, height: 720 } });

    test("desktop hides the mobile tab bar and shows the rail @mobile:issue-detail", async ({
      authenticatedPage: page,
    }) => {
      await page.goto("/issues/i-seed00001");

      // MobileTabBar is rendered but hidden via CSS at desktop widths.
      await expect(page.getByTestId("issue-mobile-tab-overview")).toBeHidden();
      await expect(page.getByTestId("issue-mobile-tab-related")).toBeHidden();
      await expect(page.getByTestId("issue-mobile-tab-activity")).toBeHidden();
      await expect(page.getByTestId("issue-mobile-tab-details")).toBeHidden();

      // The right-rail sub-tabs are visible at desktop.
      await expect(page.getByTestId("issue-rail-tab-related")).toBeVisible();
      await expect(page.getByTestId("issue-rail-tab-activity")).toBeVisible();
      await expect(page.getByTestId("issue-rail-tab-details")).toBeVisible();

      // Breadcrumb title visible alongside rail. Switching to Details
      // surfaces the status chip + Created label (those live in the Details
      // tab now).
      await expect(
        page.locator('nav[aria-label="Breadcrumb"]').getByText(
          "Platform v2.0 Migration",
        ),
      ).toBeVisible();
      await page.getByTestId("issue-rail-tab-details").click();
      await expect(page.getByTestId("status-chip")).toBeVisible();
      // Scope to the rail since SessionList also renders a "Created" column header.
      await expect(
        page.getByRole("complementary").getByText("Created", { exact: true }),
      ).toBeVisible();
    });
  });
});
