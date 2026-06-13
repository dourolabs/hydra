import { test, expect } from "../fixtures/auth";

test.describe("Issue Session Settings editor @issues:edit-session-settings", () => {
  test("edits max_retries and round-trips through updateIssue", async ({
    authenticatedPage: page,
  }) => {
    // i-seed00002 seeds session_settings = { repo_name: "acme/web-app" }.
    // max_retries starts unset (renders as "Agent default").
    await page.goto("/issues/i-seed00002");
    await page.getByTestId("issue-rail-tab-details").click();

    // Display rows reflect the seeded settings.
    await expect(page.getByText("Session settings")).toBeVisible();
    const display = page.getByTestId("issue-settings-display");
    await expect(display).toBeVisible();
    await expect(display.getByText("acme/web-app")).toBeVisible();
    // max_retries row exists and shows the Agent-default placeholder.
    await expect(
      page.getByTestId("issue-settings-row-max_retries"),
    ).toContainText("Agent default");

    // Enter edit mode.
    await page.getByTestId("issue-settings-edit").click();
    const editor = page.getByTestId("issue-settings-editor");
    await expect(editor).toBeVisible();

    // The repo_name input is prefilled.
    const repoInput = page.getByTestId("issue-settings-input-repo_name");
    await expect(repoInput).toHaveValue("acme/web-app");

    // Set max_retries to 7 and a model override.
    const retriesInput = page.getByTestId("issue-settings-input-max_retries");
    await retriesInput.fill("7");

    const modelInput = page.getByTestId("issue-settings-input-model");
    await modelInput.fill("opus-4.7");

    await page.getByTestId("issue-settings-save").click();

    // Editor should close and display the new values.
    await expect(page.getByTestId("issue-settings-editor")).toHaveCount(0);
    const displayAfter = page.getByTestId("issue-settings-display");
    await expect(displayAfter).toBeVisible();
    await expect(
      page.getByTestId("issue-settings-row-max_retries"),
    ).toContainText("7");
    await expect(page.getByTestId("issue-settings-row-model")).toContainText(
      "opus-4.7",
    );
    // repo_name is preserved on the round trip.
    await expect(
      page.getByTestId("issue-settings-row-repo_name"),
    ).toContainText("acme/web-app");
  });

  test("clearing a field restores the agent default", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00002");
    await page.getByTestId("issue-rail-tab-details").click();

    const retriesRow = page.getByTestId("issue-settings-row-max_retries");

    // First set max_retries.
    await page.getByTestId("issue-settings-edit").click();
    await page.getByTestId("issue-settings-input-max_retries").fill("5");
    await page.getByTestId("issue-settings-save").click();
    await expect(page.getByTestId("issue-settings-editor")).toHaveCount(0);
    await expect(retriesRow).toContainText("5");

    // Now clear it.
    await page.getByTestId("issue-settings-edit").click();
    await page.getByTestId("issue-settings-input-max_retries").fill("");
    await page.getByTestId("issue-settings-save").click();
    await expect(page.getByTestId("issue-settings-editor")).toHaveCount(0);

    // Max retries row should be back to the agent-default placeholder.
    await expect(retriesRow).toContainText("Agent default");
  });

  test("rejects a non-integer max_retries value with an inline error", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00002");
    await page.getByTestId("issue-rail-tab-details").click();

    await page.getByTestId("issue-settings-edit").click();
    // The input is type=number so most browsers strip non-numeric chars; assign
    // a negative value to trip the validation guard instead.
    await page.getByTestId("issue-settings-input-max_retries").fill("-1");
    await page.getByTestId("issue-settings-save").click();

    await expect(page.getByTestId("issue-settings-error")).toBeVisible();
    // Editor stays open.
    await expect(page.getByTestId("issue-settings-editor")).toBeVisible();
  });
});
