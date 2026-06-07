import { test, expect } from "../../fixtures/auth";
import type { Page } from "@playwright/test";

// On mobile, the sidebar drawer is open by default and auto-closes after
// any navigation link is tapped (see Sidebar.tsx handleNavClick). Tests that
// start by tapping the hamburger to open the drawer must persist "hidden"
// before navigation so the drawer is closed on first load.
async function setSidebarHidden(page: Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem("hydra-sidebar-hidden", "1");
  });
}

async function openMobileDrawer(page: Page): Promise<void> {
  await page.getByTestId("site-header-toggle-sidebar").click();
  await expect(page.getByTestId("sidebar-backdrop")).toBeVisible();
}

test.describe("Mobile Navigation @mobile:nav", () => {
  test("sidebar links navigate to correct pages @mobile:nav", async ({
    authenticatedPage: page,
  }) => {
    await setSidebarHidden(page);
    await page.goto("/");

    // Navigate to Documents via the Documents workspace entry.
    await openMobileDrawer(page);
    await page.getByTestId("sidebar-documents").click();
    await expect(page).toHaveURL(/\/documents/);

    // Navigate to the Agents page via the Agents sidebar entry.
    await openMobileDrawer(page);
    await page.getByTestId("sidebar-agents").click();
    await expect(page).toHaveURL(/\/agents/);

    // Navigate to the all-issues dashboard via Workspace > Issues.
    await openMobileDrawer(page);
    await page.getByTestId("sidebar-issues-all").click();
    await expect(page).toHaveURL(/^http:\/\/localhost:\d+\/$/);
  });

  test("deep link to issue detail renders correctly @mobile:nav", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00001");
    // Breadcrumb shows the issue ID as the trailing crumb; the title is the heading.
    await expect(
      page.locator('nav[aria-label="Breadcrumb"]').getByText("i-seed00001")
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Platform v2.0 Migration" })
    ).toBeVisible();
  });

  test("deep link to patch detail renders correctly @mobile:nav", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/patches/p-seed00001");
    await expect(
      page.getByRole("heading", { name: "Add OAuth2 provider integration" })
    ).toBeVisible();
  });
});
