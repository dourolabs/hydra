import { test, expect } from "../fixtures/auth";

// ProjectEditor (in the project Settings modal) uses the inline
// PromptDocumentEditor for both the project's own prompt and each per-status
// prompt. Collapsed = just the path Input. Expanding the toggle reveals a
// textarea backed by the docs API. The simplified new-project modal no
// longer uses this editor, so the test opens the Settings modal of a
// seeded project instead.
test.describe("ProjectEditor prompt-document-editor @projects:prompt-editor", () => {
  test("project settings modal exposes the inline prompt editor", async ({
    authenticatedPage: page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto("/projects");
    await page.getByTestId("board-project-settings-engineering-v2").click();

    const modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();

    // 1) Collapsed state — only the path Input is rendered.
    const projectPath = page.getByTestId("project-editor-prompt-path");
    await expect(projectPath).toBeVisible();
    await expect(
      page.getByTestId("project-editor-prompt-path-textarea"),
    ).toHaveCount(0);

    // 2) Toggling expands the editor (renders the textarea).
    await page.getByTestId("project-editor-prompt-path-toggle").click();
    await expect(
      page.getByTestId("project-editor-prompt-path-textarea"),
    ).toBeVisible();

    // 3) Same pattern on the per-status row.
    await expect(
      page.getByTestId("status-editor-prompt-path-0"),
    ).toBeVisible();
    await expect(
      page.getByTestId("status-editor-prompt-path-0-textarea"),
    ).toHaveCount(0);
    await page.getByTestId("status-editor-prompt-path-0-toggle").click();
    await expect(
      page.getByTestId("status-editor-prompt-path-0-textarea"),
    ).toBeVisible();
  });
});
