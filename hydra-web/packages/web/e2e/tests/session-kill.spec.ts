import { test, expect } from "../fixtures/auth";

test.describe("Session Kill @sessions:kill", () => {
  test("can kill a running session @sessions:kill", async ({
    authenticatedPage: page,
  }) => {
    // Navigate to a session detail page for a running session (t-seed00002 is "running")
    await page.goto("/issues/i-seed00002/sessions/t-seed00002/logs");

    // Kill button should be visible for a running session
    const killButton = page.getByRole("button", { name: "Kill Session" });
    await expect(killButton).toBeVisible();

    // Click the kill button
    await killButton.click();

    // Confirmation modal should appear
    const modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();
    await expect(modal.getByText(/terminate the running session/)).toBeVisible();

    // Confirm the kill action
    await modal.getByRole("button", { name: "Kill Session" }).click();

    // Modal should close
    await expect(modal).not.toBeVisible();

    // Success toast should appear
    await expect(page.getByText(/Session killed successfully/)).toBeVisible();

    // Terminating spinner should appear while session status is still "running"
    await expect(page.getByText("Terminating...")).toBeVisible();

    // Kill Session button should not be visible while terminating
    await expect(killButton).not.toBeVisible();
  });

  test("cancel does not kill the session @sessions:kill", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00002/sessions/t-seed00002/logs");

    const killButton = page.getByRole("button", { name: "Kill Session" });
    await killButton.click();

    const modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();

    // Click Cancel
    await modal.getByRole("button", { name: "Cancel" }).click();

    // Modal should close
    await expect(modal).not.toBeVisible();

    // Kill button should still be visible (session still running)
    await expect(killButton).toBeVisible();
  });

  test("kill button is not visible for completed sessions @sessions:kill", async ({
    authenticatedPage: page,
  }) => {
    // t-seed00001 is a completed session
    await page.goto("/issues/i-seed00005/sessions/t-seed00001/logs");

    // Kill button should not be present
    await expect(
      page.getByRole("button", { name: "Kill Session" })
    ).not.toBeVisible();
  });
});
