import { test, expect } from "../fixtures/auth";

// Agent-level default session settings ride on the AgentEditModal (Configure
// on an agent card). The merge chain at spawn is
// Issue → Status → Project → Agent → defaults, so setting cpu / memory on
// the chat or pm agent is meant to apply to every session for that agent
// without per-issue / per-project overrides.
test.describe("Agents — default session settings @agents:session-settings", () => {
  test("filling cpu_limit + memory_limit fires PUT with nested session_settings and round-trips on reload @agents:session-settings", async ({
    authenticatedPage: page,
  }) => {
    const updatePayloads: Array<{ url: string; body: unknown }> = [];
    page.on("request", (req) => {
      const url = new URL(req.url());
      if (
        req.method() === "PUT" &&
        /\/api\/v1\/agents\/[^/]+$/.test(url.pathname)
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

    await page.goto("/agents");

    // Open the swe agent's Configure modal.
    await page.getByRole("button", { name: "Configure swe" }).click();

    const modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();

    await modal
      .getByTestId("agent-edit-form-session-settings-toggle")
      .click();
    await modal.getByTestId("agent-edit-form-cpu-limit").fill("200m");
    await modal.getByTestId("agent-edit-form-memory-limit").fill("512Mi");

    await modal.getByRole("button", { name: "Save Changes" }).click();
    await expect(modal).toBeHidden();

    await expect.poll(() => updatePayloads.length).toBeGreaterThanOrEqual(1);
    const sent = updatePayloads.find(
      (p) => (p.body as { session_settings?: unknown }).session_settings != null,
    );
    expect(sent).toBeDefined();
    expect(
      (sent!.body as { session_settings: Record<string, unknown> })
        .session_settings,
    ).toMatchObject({
      cpu_limit: "200m",
      memory_limit: "512Mi",
    });

    // Reopen — the inputs should have been hydrated from the persisted
    // agent on reload.
    await page.getByRole("button", { name: "Configure swe" }).click();
    await expect(modal).toBeVisible();
    await modal
      .getByTestId("agent-edit-form-session-settings-toggle")
      .click();
    await expect(modal.getByTestId("agent-edit-form-cpu-limit")).toHaveValue(
      "200m",
    );
    await expect(
      modal.getByTestId("agent-edit-form-memory-limit"),
    ).toHaveValue("512Mi");
  });
});
