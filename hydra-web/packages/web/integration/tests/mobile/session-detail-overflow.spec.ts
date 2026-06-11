import { test, expect } from "../../fixtures/auth";

// At mobile widths the sidebar drawer auto-opens by default and its backdrop
// intercepts pointer events on the page content. Pin it closed before
// navigation so the overflow trigger receives our click.
async function setSidebarHidden(page: import("@playwright/test").Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem("hydra-sidebar-hidden", "1");
  });
}

test.describe("Mobile session detail overflow @mobile:session-detail", () => {
  test("Kill Session lives behind the ⋯ overflow menu on mobile @mobile:session-detail", async ({
    authenticatedPage: page,
  }) => {
    await setSidebarHidden(page);
    // t-seed00002 is a running session under i-seed00002 — the only one for
    // which the kill control is conditionally rendered.
    await page.goto("/issues/i-seed00002/sessions/t-seed00002/logs");

    // The inline Kill Session button must NOT be visible at mobile widths;
    // mobile collapses it into the overflow menu.
    await expect(
      page.getByRole("button", { name: "Kill Session" }),
    ).not.toBeVisible();

    // The status badge stays inline next to the creator line — assert it's
    // still on screen so we haven't accidentally hidden load-bearing chrome.
    // The session is "running", so look for that status string in the header.
    const headerStatus = page
      .locator("header, [class*='header']")
      .filter({ hasText: /running/i })
      .first();
    await expect(headerStatus).toBeVisible();

    const trigger = page.getByTestId("session-overflow-trigger");
    await expect(trigger).toBeVisible();
    await trigger.click();

    const killItem = page.getByTestId("session-overflow-kill");
    await expect(killItem).toBeVisible();
    await killItem.click();

    // Confirmation modal opens; confirm to fire the kill — verifies the
    // overflow path wires through to the same mutation as desktop.
    const modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();
    await modal.getByRole("button", { name: "Kill Session" }).click();

    await expect(modal).not.toBeVisible();
    await expect(page.getByText(/Session killed successfully/)).toBeVisible();
  });

  test("overflow trigger is absent for completed sessions on mobile @mobile:session-detail", async ({
    authenticatedPage: page,
  }) => {
    await setSidebarHidden(page);
    // t-seed00001 is completed — nothing to kill, so the overflow trigger
    // shouldn't render either.
    await page.goto("/issues/i-seed00005/sessions/t-seed00001/logs");

    await expect(page.getByTestId("session-overflow-trigger")).toHaveCount(0);
    await expect(
      page.getByRole("button", { name: "Kill Session" }),
    ).not.toBeVisible();
  });
});
