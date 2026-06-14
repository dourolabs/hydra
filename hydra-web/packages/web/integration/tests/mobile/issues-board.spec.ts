import { test, expect } from "../../fixtures/auth";
import type { Page } from "@playwright/test";

async function setSidebarHidden(page: Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem("hydra-sidebar-hidden", "1");
  });
}

test.describe("Mobile Issues Board @mobile:issues-board", () => {
  test("board view shows a single project + picker @mobile:issues-board", async ({
    authenticatedPage: page,
  }) => {
    await setSidebarHidden(page);
    await page.goto("/");

    // Mobile forces the board layout — no segmented control is rendered, so
    // we land on the board directly.
    await expect(page.getByTestId("issues-layout-board")).toHaveCount(0);

    // The mobile board picker must appear when more than one project exists.
    await expect(page.getByTestId("board-mobile-picker")).toBeVisible();

    // Only one project section should be mounted at a time. The `<section>`
    // root carries `data-testid="board-project-<key>"`; nested
    // `board-project-bar-…` / `board-project-body-…` testids live on inner
    // divs, so scoping by tag isolates the section root regardless of
    // `hideBar`.
    const sections = page.locator('section[data-testid^="board-project-"]');
    await expect(sections).toHaveCount(1);
  });

  test("picker selection persists across navigations @mobile:issues-board", async ({
    authenticatedPage: page,
  }) => {
    await setSidebarHidden(page);
    await page.goto("/");

    const picker = page.getByTestId("board-mobile-picker");
    await expect(picker).toBeVisible();

    // Open the picker and pick the second option so we can confirm
    // round-tripping isn't trivially matched by the default.
    await picker.getByRole("button", { name: "Board" }).click();
    const options = page.locator(
      '[data-testid^="board-mobile-picker-option-"]',
    );
    const count = await options.count();
    if (count < 2) {
      test.skip(true, "Need at least two seeded projects for this assertion");
    }
    // Capture the row's label *before* clicking — the click closes the picker
    // and unmounts the row, so reading textContent after would return null.
    const initiallySelectedLabel = (
      await options.nth(1).textContent()
    )?.trim();
    await options.nth(1).click();

    // Navigate away and back; the selection should be restored.
    await page.goto("/sessions");
    await page.goto("/");

    // The picker trigger shows the currently selected board — its text
    // should match the option we picked before navigating away.
    await expect(
      page.getByTestId("board-mobile-picker").getByRole("button"),
    ).toContainText(initiallySelectedLabel ?? "");
  });
});
