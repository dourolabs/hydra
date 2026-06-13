import { test, expect } from "../fixtures/auth";

test.describe("Header create menu @nav:header-create-menu", () => {
  test("opens menu and creates a new issue @nav:header-create-menu", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=all");
    await expect(page.getByText("Platform v2.0 Migration")).toBeVisible();

    const trigger = page.getByTestId("site-header-create");
    await expect(trigger).toHaveAttribute("aria-haspopup", "menu");
    await expect(trigger).toHaveAttribute("aria-expanded", "false");

    await trigger.click();

    const menu = page.getByTestId("site-header-create-menu");
    await expect(menu).toBeVisible();
    await expect(trigger).toHaveAttribute("aria-expanded", "true");
    await expect(page.getByTestId("site-header-new-issue")).toBeVisible();
    await expect(page.getByTestId("site-header-new-conversation")).toBeVisible();

    await page.getByTestId("site-header-new-issue").click();

    // Menu closes and the create-issue modal opens.
    await expect(menu).toBeHidden();
    await expect(page.getByTestId("issue-create-modal")).toBeVisible();
  });

  test("opens the create-conversation modal @nav:header-create-menu", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=all");
    await expect(page.getByText("Platform v2.0 Migration")).toBeVisible();

    await page.getByTestId("site-header-create").click();
    await page.getByTestId("site-header-new-conversation").click();

    await expect(page.getByTestId("site-header-create-menu")).toBeHidden();
    await expect(page.getByTestId("chat-create-modal")).toBeVisible();
  });

  test("closes the menu on Escape @nav:header-create-menu", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=all");

    await page.getByTestId("site-header-create").click();
    await expect(page.getByTestId("site-header-create-menu")).toBeVisible();

    await page.keyboard.press("Escape");
    await expect(page.getByTestId("site-header-create-menu")).toBeHidden();
  });

  test("closes the menu on outside click @nav:header-create-menu", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=all");

    await page.getByTestId("site-header-create").click();
    await expect(page.getByTestId("site-header-create-menu")).toBeVisible();

    // Click in the breadcrumbs area, outside the menu.
    await page.getByTestId("site-header-breadcrumbs").click({ force: true });
    await expect(page.getByTestId("site-header-create-menu")).toBeHidden();
  });

  test("create menu is hidden on mobile (replaced by section FABs) @nav:header-create-menu", async ({
    authenticatedPage: page,
  }) => {
    // Start with the mobile drawer dismissed so the backdrop doesn't intercept clicks.
    await page.addInitScript(() => {
      try {
        window.localStorage.setItem("hydra-sidebar-hidden", "1");
      } catch {
        /* ignore */
      }
    });
    await page.setViewportSize({ width: 375, height: 812 });
    await page.goto("/?selected=all");
    await expect(page.getByTestId("sidebar-backdrop")).toBeHidden();

    // The desktop create-menu trigger is not rendered on mobile.
    await expect(page.getByTestId("site-header-create")).toHaveCount(0);
  });
});
