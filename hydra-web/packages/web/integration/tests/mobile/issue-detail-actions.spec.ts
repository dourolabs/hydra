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

test.describe("Mobile Issue Detail head actions @mobile:issue-detail-actions", () => {
  test.use({ viewport: { width: 375, height: 812 } });

  test("inline head action buttons are hidden behind the overflow trigger @mobile:issue-detail-actions", async ({
    authenticatedPage: page,
  }) => {
    await setSidebarHidden(page);
    await page.goto("/issues/i-seed00002");

    // The inline desktop action row collapses to a `⋯` button.
    await expect(page.getByTestId("issue-overflow-trigger")).toBeVisible();
    await expect(page.getByTestId("issue-detail-archive")).toBeHidden();
    await expect(
      page.getByRole("button", { name: "Give feedback" }),
    ).toBeHidden();

    // Tapping the trigger reveals Give feedback + Archive menu items.
    await page.getByTestId("issue-overflow-trigger").click();
    await expect(page.getByTestId("issue-overflow-feedback")).toBeVisible();
    await expect(page.getByTestId("issue-overflow-archive")).toBeVisible();
  });

  test("Resume / Open Conversation surfaces in the overflow menu when present @mobile:issue-detail-actions", async ({
    authenticatedPage: page,
  }) => {
    await setSidebarHidden(page);
    // i-seed00001 has an active spawned conversation; the menu carries an
    // "Open Conversation" entry that deep-links to /chat/<id>.
    await page.goto("/issues/i-seed00001");

    await page.getByTestId("issue-overflow-trigger").click();
    const conversationItem = page.getByTestId("issue-overflow-conversation");
    await expect(conversationItem).toBeVisible();
    await expect(conversationItem).toHaveText("Open Conversation");

    await conversationItem.click();
    await expect(page).toHaveURL(/\/chat\/c-seed00008$/);
  });
});

test.describe("Desktop Issue Detail head actions are unchanged @mobile:issue-detail-actions", () => {
  test.use({ viewport: { width: 1280, height: 720 } });

  test("inline buttons render and the overflow trigger is hidden @mobile:issue-detail-actions", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00002");

    // Desktop keeps the inline row exactly as before.
    await expect(page.getByTestId("issue-detail-archive")).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Give feedback" }),
    ).toBeVisible();

    // The overflow trigger renders but is hidden by the mobile media query.
    await expect(page.getByTestId("issue-overflow-trigger")).toBeHidden();
  });
});
