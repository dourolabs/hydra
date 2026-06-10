import { test, expect } from "../fixtures/auth";

// `ProjectEditor` (rendered on the project detail page `/projects/<key>`)
// uses the inline `PromptDocumentEditor` for both the project's own prompt
// and each per-status prompt. Collapsed = just the path Input. Expanding
// the toggle reveals a textarea backed by the docs API.
//
// The simplified board-level Settings modal (`ProjectForm`) no longer uses
// `PromptDocumentEditor`, so this spec drives the detail page where the
// full editor still lives.
test.describe("ProjectEditor prompt-document-editor @projects:prompt-editor", () => {
  test("project detail page exposes the inline prompt editor", async ({
    authenticatedPage: page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto("/projects/engineering-v2");

    await expect(page.getByTestId("project-editor")).toBeVisible();

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
