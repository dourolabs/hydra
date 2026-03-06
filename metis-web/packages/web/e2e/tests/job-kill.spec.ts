import { test, expect } from "../fixtures/auth";

test.describe("Job Kill @jobs:kill", () => {
  test("can kill a running job @jobs:kill", async ({
    authenticatedPage: page,
  }) => {
    // Navigate to a job detail page for a running job (t-seed00002 is "running")
    await page.goto("/issues/i-seed00002/jobs/t-seed00002/logs");

    // Kill button should be visible for a running job
    const killButton = page.getByRole("button", { name: "Kill Job" });
    await expect(killButton).toBeVisible();

    // Click the kill button
    await killButton.click();

    // Confirmation modal should appear
    const modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();
    await expect(modal.getByText(/terminate the running job/)).toBeVisible();

    // Confirm the kill action
    await modal.getByRole("button", { name: "Kill Job" }).click();

    // Modal should close
    await expect(modal).not.toBeVisible();

    // Success toast should appear
    await expect(page.getByText(/Job killed successfully/)).toBeVisible();
  });

  test("cancel does not kill the job @jobs:kill", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00002/jobs/t-seed00002/logs");

    const killButton = page.getByRole("button", { name: "Kill Job" });
    await killButton.click();

    const modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();

    // Click Cancel
    await modal.getByRole("button", { name: "Cancel" }).click();

    // Modal should close
    await expect(modal).not.toBeVisible();

    // Kill button should still be visible (job still running)
    await expect(killButton).toBeVisible();
  });

  test("kill button is not visible for completed jobs @jobs:kill", async ({
    authenticatedPage: page,
  }) => {
    // t-seed00001 is a completed job
    await page.goto("/issues/i-seed00005/jobs/t-seed00001/logs");

    // Kill button should not be present
    await expect(
      page.getByRole("button", { name: "Kill Job" })
    ).not.toBeVisible();
  });
});
