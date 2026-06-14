import { test, expect } from "../fixtures/auth";

// Project-level default session settings ride on the ProjectSettingsModal
// (gear icon on a project bar) — the merge chain at spawn is
// Issue → Status → Project → defaults, so setting an image / cpu on a
// project is meant to apply to every issue under it without per-status
// overrides.
test.describe("Project settings — default session settings @projects:session-settings", () => {
  test("filling cpu_limit + image fires PUT with nested session_settings and round-trips on reload @projects:session-settings", async ({
    authenticatedPage: page,
  }) => {
    const updatePayloads: Array<{ url: string; body: unknown }> = [];
    page.on("request", (req) => {
      const url = new URL(req.url());
      if (
        req.method() === "PUT" &&
        /\/api\/v1\/projects\/[^/]+$/.test(url.pathname)
      ) {
        try {
          updatePayloads.push({
            url: url.pathname,
            body: JSON.parse(req.postData() ?? "null"),
          });
        } catch {
          /* ignore */
        }
      }
    });

    await page.goto("/?selected=all");
    await page.getByTestId("issues-layout-board").click();

    // Open the engineering-v2 project's ProjectSettingsModal via the gear.
    const settingsBtn = page.getByTestId(
      "board-project-settings-engineering-v2",
    );
    await expect(settingsBtn).toBeVisible();
    await settingsBtn.click();

    const modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();

    await modal
      .getByTestId("project-form-session-settings-toggle")
      .click();
    await modal
      .getByTestId("project-form-cpu-limit")
      .fill("750m");
    await modal
      .getByTestId("project-form-image")
      .fill("ghcr.io/org/img:tag");

    await modal.getByTestId("project-form-save").click();
    await expect(modal).toBeHidden();

    await expect.poll(() => updatePayloads.length).toBeGreaterThanOrEqual(1);
    const sent = updatePayloads.find((p) =>
      (p.body as { session_settings?: unknown }).session_settings != null,
    );
    expect(sent).toBeDefined();
    expect(
      (sent!.body as { session_settings: Record<string, unknown> })
        .session_settings,
    ).toMatchObject({
      cpu_limit: "750m",
      image: "ghcr.io/org/img:tag",
    });

    // Reopen — the inputs should have been hydrated from the persisted
    // project on reload.
    await settingsBtn.click();
    await expect(modal).toBeVisible();
    await modal
      .getByTestId("project-form-session-settings-toggle")
      .click();
    await expect(modal.getByTestId("project-form-cpu-limit")).toHaveValue(
      "750m",
    );
    await expect(modal.getByTestId("project-form-image")).toHaveValue(
      "ghcr.io/org/img:tag",
    );
  });
});
