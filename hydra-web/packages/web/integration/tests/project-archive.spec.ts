import { test, expect } from "../fixtures/auth";

// Phase 4: project archive surfaces an "N issues will be archived" hint
// in the confirmation dialog before invoking the per-project archive
// route. The mock server cascades the archive to every non-archived
// issue in the project; the board re-renders without the archived
// project's section.

test.describe("Project settings — Archive project @projects:archive", () => {
  test("archiving a project from its settings modal hides its board section after confirm @projects:archive", async ({
    authenticatedPage: page,
  }) => {
    const archiveCalls: string[] = [];
    page.on("request", (req) => {
      const url = new URL(req.url());
      if (
        req.method() === "POST" &&
        /\/api\/v1\/projects\/[^/]+\/archive$/.test(url.pathname) &&
        !url.pathname.endsWith("/statuses/")
      ) {
        archiveCalls.push(url.pathname);
      }
    });

    await page.goto("/?selected=all");
    await page.getByTestId("issues-layout-board").click();

    // The `engineering-v2` seed project has its own section; click its
    // settings (gear) button to open ProjectSettingsModal.
    const settingsBtn = page.getByTestId(
      "board-project-settings-engineering-v2",
    );
    await expect(settingsBtn).toBeVisible();
    await settingsBtn.click();

    const modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();

    // Trigger the archive confirmation.
    await modal.getByTestId("project-form-delete").click();
    // The confirmation modal renders inside its own dialog.
    const confirmDialog = page.getByRole("dialog", { name: /Archive Project/ });
    await expect(confirmDialog).toBeVisible();

    await confirmDialog.getByRole("button", { name: "Archive" }).click();

    await expect
      .poll(() =>
        archiveCalls.some((p) => p.includes("engineering-v2") || p.includes("j-engv2")),
      )
      .toBe(true);

    // The archived project's section is gone; the default project's is
    // still rendered.
    await expect(
      page.getByTestId("board-project-bar-engineering-v2"),
    ).toHaveCount(0);
    await expect(page.getByTestId("board-project-bar-default")).toBeVisible();
  });
});
