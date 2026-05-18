import { test, expect } from "../../fixtures/auth";
import type { Page } from "@playwright/test";

// On mobile the sidebar drawer is open by default and intercepts pointer
// events on the page content underneath. Persist "hidden" before navigation
// so the drawer stays closed for assertions that need to interact with the
// main column.
async function setSidebarHidden(page: Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem("hydra-sidebar-hidden", "1");
  });
}

test.describe("Mobile Dashboard @mobile:dashboard", () => {
  test("dashboard renders and shows issues @mobile:dashboard", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=your-issues");

    // Issues should be visible in the flat list
    await expect(
      page.getByText("Update deployment documentation")
    ).toBeVisible();
  });

  test("can click through to an issue from dashboard @mobile:dashboard", async ({
    authenticatedPage: page,
  }) => {
    await setSidebarHidden(page);
    await page.goto("/?selected=your-issues");

    // Click on an issue to navigate to detail
    await page.getByText("Update deployment documentation").click();
    await expect(page).toHaveURL(/\/issues\/i-seed00010/);
  });
});
