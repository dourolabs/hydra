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

    await page.getByTestId("issues-layout-board").click();

    // The mobile board picker must appear when more than one project exists.
    await expect(page.getByTestId("board-mobile-picker")).toBeVisible();

    // Only one project section should be mounted at a time. `board-project-bar-…`
    // is on the project bar so its count is the section count.
    const bars = page.locator('[data-testid^="board-project-bar-"]');
    await expect(bars).toHaveCount(1);
  });

  test("picker selection persists across navigations @mobile:issues-board", async ({
    authenticatedPage: page,
  }) => {
    await setSidebarHidden(page);
    await page.goto("/");
    await page.getByTestId("issues-layout-board").click();

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
    const secondTestId = await options.nth(1).getAttribute("data-testid");
    if (!secondTestId) throw new Error("missing data-testid on picker row");
    await options.nth(1).click();
    const initiallySelectedLabel = (
      await page.getByTestId(secondTestId).textContent()
    )?.trim();

    // Navigate away and back; the selection should be restored.
    await page.goto("/sessions");
    await page.goto("/");
    await page.getByTestId("issues-layout-board").click();

    // The picker trigger shows the currently selected board — its text
    // should match the option we picked before navigating away.
    await expect(
      page.getByTestId("board-mobile-picker").getByRole("button"),
    ).toContainText(initiallySelectedLabel ?? "");
  });
});
