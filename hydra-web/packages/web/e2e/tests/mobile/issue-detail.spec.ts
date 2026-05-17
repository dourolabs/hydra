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

    test("displays issue heading and breadcrumbs @mobile:issue-detail", async ({
      authenticatedPage: page,
    }) => {
      await setSidebarHidden(page);
      await page.goto("/issues/i-seed00002");

      // Breadcrumb shows the issue ID as the trailing crumb.
      const breadcrumb = page.locator('nav[aria-label="Breadcrumb"]');
      await expect(breadcrumb).toBeVisible();
      await expect(breadcrumb.getByText("i-seed00002")).toBeVisible();

      const title = page.getByRole("heading", {
        name: "Migrate authentication to OAuth2",
      });
      await expect(title).toBeVisible();
      await expect(title).toBeInViewport();

      // Each pane owns its own scroll — the page body must not have scrolled.
      const docScroll = await page.evaluate(
        () => document.scrollingElement?.scrollTop ?? window.scrollY,
      );
      expect(docScroll).toBe(0);
    });

    test("top tabs toggle between Overview and Details @mobile:issue-detail", async ({
      authenticatedPage: page,
    }) => {
      await setSidebarHidden(page);
      await page.goto("/issues/i-seed00001");

      const overviewTab = page.getByTestId("issue-mobile-tab-overview");
      const detailsTab = page.getByTestId("issue-mobile-tab-details");

      await expect(overviewTab).toBeVisible();
      await expect(detailsTab).toBeVisible();
      await expect(overviewTab).toHaveAttribute("aria-selected", "true");
      await expect(detailsTab).toHaveAttribute("aria-selected", "false");

      // Overview pane shows the title heading and the inline sub-tabs row.
      const heading = page.getByRole("heading", { name: "Platform v2.0 Migration" });
      await expect(heading).toBeVisible();
      await expect(page.getByTestId("issue-tab-sessions")).toBeVisible();
      await expect(page.getByTestId("issue-tab-patches")).toBeVisible();
      await expect(page.getByTestId("issue-tab-activity")).toBeVisible();
      await expect(page.getByTestId("issue-tab-sub-issues")).toBeVisible();

      // The status chip lives in the rail — hidden until Details is active.
      const statusChip = page.getByTestId("status-chip");
      await expect(statusChip).toBeHidden();

      // Switch to Details — rail content visible, Overview hidden.
      await detailsTab.click();
      await expect(detailsTab).toHaveAttribute("aria-selected", "true");
      await expect(overviewTab).toHaveAttribute("aria-selected", "false");
      await expect(heading).toBeHidden();
      await expect(page.getByTestId("issue-tab-sessions")).toBeHidden();

      // Rail content (status chip, "Created" block label) is visible.
      await expect(statusChip).toBeVisible();
      await expect(page.getByText("Created", { exact: true })).toBeVisible();

      // Switch back to Overview — heading is back, rail hidden.
      await overviewTab.click();
      await expect(overviewTab).toHaveAttribute("aria-selected", "true");
      await expect(heading).toBeVisible();
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
      await expect(page.getByTestId("issue-mobile-tab-details")).toBeHidden();

      // Heading visible alongside rail content (status chip, Created label).
      await expect(
        page.getByRole("heading", { name: "Platform v2.0 Migration" }),
      ).toBeVisible();
      await expect(page.getByTestId("status-chip")).toBeVisible();
      await expect(page.getByText("Created", { exact: true })).toBeVisible();
    });
  });
});
