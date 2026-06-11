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

    // Pick the second option so we can confirm round-tripping isn't trivially
    // matched by the default.
    const optionValues = await picker.locator("option").evaluateAll((els) =>
      (els as HTMLOptionElement[]).map((o) => o.value),
    );
    if (optionValues.length < 2) {
      test.skip(true, "Need at least two seeded projects for this assertion");
    }
    await picker.selectOption(optionValues[1]);

    // Navigate away and back; the selection should be restored.
    await page.goto("/sessions");
    await page.goto("/");
    await page.getByTestId("issues-layout-board").click();

    await expect(page.getByTestId("board-mobile-picker")).toHaveValue(
      optionValues[1],
    );
  });
});
