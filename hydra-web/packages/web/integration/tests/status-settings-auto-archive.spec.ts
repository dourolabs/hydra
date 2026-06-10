import { test, expect } from "../fixtures/auth";

test.describe("Status settings — Auto-archive after @projects:status", () => {
  test("setting a value + unit persists via the per-status PUT and inverse-renders on reload @projects:status", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/?selected=all");
    await page.getByTestId("issues-layout-board").click();

    // The gear icon is `visibility: hidden` until column hover, mirroring
    // the existing add-issue button pattern.
    const openHead = page.getByTestId("board-col-head-default-open");
    await openHead.hover();
    await page.getByTestId("board-col-gear-default-open").click();

    const modal = page.getByRole("dialog", { name: /^Status — Open/ });
    await expect(modal).toBeVisible();

    const value = modal.getByTestId("status-settings-auto-archive-value");
    const unit = modal.getByTestId("status-settings-auto-archive-unit");
    await expect(value).toHaveValue("");
    await expect(unit).toHaveValue("days");

    await value.fill("14");
    await modal.getByTestId("status-settings-save").click();
    await expect(modal).toBeHidden();

    // Reopen — 14 days = 1209600s = 2 weeks; the inverse-render rule prefers
    // weeks so the round-tripped value doesn't bloat into "336 hours".
    await openHead.hover();
    await page.getByTestId("board-col-gear-default-open").click();
    await expect(modal).toBeVisible();
    await expect(
      modal.getByTestId("status-settings-auto-archive-value"),
    ).toHaveValue("2");
    await expect(
      modal.getByTestId("status-settings-auto-archive-unit"),
    ).toHaveValue("weeks");

    // Clear and save — back to empty.
    await modal.getByTestId("status-settings-auto-archive-value").fill("");
    await modal.getByTestId("status-settings-save").click();
    await expect(modal).toBeHidden();

    await openHead.hover();
    await page.getByTestId("board-col-gear-default-open").click();
    await expect(modal).toBeVisible();
    await expect(
      modal.getByTestId("status-settings-auto-archive-value"),
    ).toHaveValue("");
  });
});
