import { test, expect } from "../../fixtures/auth";
import type { Page } from "@playwright/test";

// On mobile, the sidebar lives in a closed-by-default drawer that auto-closes
// after any navigation link is tapped (see Sidebar.tsx handleNavClick). So
// every sidebar click in a mobile test must be preceded by opening the drawer.
async function openMobileDrawer(page: Page): Promise<void> {
  await page.getByTestId("site-header-toggle-sidebar").click();
  await expect(page.getByTestId("sidebar-backdrop")).toBeVisible();
}

test.describe("Mobile Navigation @mobile:nav", () => {
  test("sidebar links navigate to correct pages @mobile:nav", async ({
    authenticatedPage: page,
  }) => {
    // Navigate to Documents via the Documents section "More" link.
    await openMobileDrawer(page);
    await page.getByTestId("sidebar-section-documents-more").click();
    await expect(page).toHaveURL(/\/documents/);

    // Navigate to the Agents page via the Agents sidebar entry.
    await openMobileDrawer(page);
    await page.getByTestId("sidebar-agents").click();
    await expect(page).toHaveURL(/\/agents/);

    // Navigate to the dashboard via the Issues > All issues link.
    await openMobileDrawer(page);
    await page.getByTestId("sidebar-issues-all").click();
    await expect(page).toHaveURL(
      /^http:\/\/localhost:\d+\/\?selected=all$/,
    );
  });

  test("deep link to issue detail renders correctly @mobile:nav", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00001");
    await expect(
      page.locator('nav[aria-label="Breadcrumb"]').getByText("Platform v2.0 Migration")
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
