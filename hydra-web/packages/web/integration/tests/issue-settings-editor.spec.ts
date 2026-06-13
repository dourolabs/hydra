import { test, expect } from "../fixtures/auth";

test.describe("Issue Session Settings editor @issues:edit-session-settings", () => {
  test("edits max_retries and round-trips through updateIssue", async ({
    authenticatedPage: page,
  }) => {
    // i-seed00002 seeds session_settings = { repo_name: "acme/web-app" }.
    // max_retries starts unset (renders as "Default").
    await page.goto("/issues/i-seed00002");
    await page.getByTestId("issue-rail-tab-details").click();

    await expect(page.getByText("Session settings")).toBeVisible();
    const repoRow = page.getByTestId("issue-settings-row-repo_name");
    await expect(repoRow).toContainText("acme/web-app");
    await expect(
      page.getByTestId("issue-settings-row-max_retries"),
    ).toContainText("Default");

    // Click the max_retries value to enter edit mode, fill, blur to save.
    await page.getByTestId("issue-settings-value-max_retries").click();
    const retriesInput = page.getByTestId("issue-settings-input-max_retries");
    await retriesInput.fill("7");
    await retriesInput.blur();

    await expect(
      page.getByTestId("issue-settings-row-max_retries"),
    ).toContainText("7");

    // Edit a second field (model) — repo_name should be preserved.
    await page.getByTestId("issue-settings-value-model").click();
    const modelInput = page.getByTestId("issue-settings-input-model");
    await modelInput.fill("opus-4.7");
    await modelInput.blur();

    await expect(page.getByTestId("issue-settings-row-model")).toContainText(
      "opus-4.7",
    );
    await expect(repoRow).toContainText("acme/web-app");
  });

  test("clearing a field restores the default", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00002");
    await page.getByTestId("issue-rail-tab-details").click();

    const retriesRow = page.getByTestId("issue-settings-row-max_retries");

    // First set max_retries to 5.
    await page.getByTestId("issue-settings-value-max_retries").click();
    await page.getByTestId("issue-settings-input-max_retries").fill("5");
    await page.getByTestId("issue-settings-input-max_retries").blur();
    await expect(retriesRow).toContainText("5");

    // Now clear it.
    await page.getByTestId("issue-settings-value-max_retries").click();
    await page.getByTestId("issue-settings-input-max_retries").fill("");
    await page.getByTestId("issue-settings-input-max_retries").blur();

    await expect(retriesRow).toContainText("Default");
  });

  test("rejects a non-integer max_retries value with an inline error", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00002");
    await page.getByTestId("issue-rail-tab-details").click();

    await page.getByTestId("issue-settings-value-max_retries").click();
    const retriesInput = page.getByTestId("issue-settings-input-max_retries");
    // type=number strips most non-numeric chars; use a negative to trip the guard.
    await retriesInput.fill("-1");
    await retriesInput.blur();

    await expect(page.getByTestId("issue-settings-error")).toBeVisible();
    // The input stays visible — row remains in edit mode for correction.
    await expect(retriesInput).toBeVisible();
  });

  test("Escape cancels an in-progress edit without saving", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00002");
    await page.getByTestId("issue-rail-tab-details").click();

    const branchRow = page.getByTestId("issue-settings-row-branch");
    await expect(branchRow).toContainText("Default");

    await page.getByTestId("issue-settings-value-branch").click();
    const branchInput = page.getByTestId("issue-settings-input-branch");
    await branchInput.fill("dev");
    await branchInput.press("Escape");

    // Edit mode closes and the value is unchanged.
    await expect(branchInput).toHaveCount(0);
    await expect(branchRow).toContainText("Default");
  });
});
