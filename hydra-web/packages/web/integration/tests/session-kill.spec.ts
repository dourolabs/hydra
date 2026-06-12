import { test, expect } from "../fixtures/auth";
import type { Page } from "@playwright/test";

type KillableStatus = "created" | "pending" | "running";

// The mock-server seed only includes `running` and `complete` sessions, so
// the pre-running (`created` / `pending`) cases are driven by overriding the
// session GET (and DELETE) for a synthetic id. The desktop kill flow is
// otherwise unchanged: optimistic update flips `status` → `"failed"`, and
// the button conditional re-evaluates against the new status.
async function mockPreRunningSession(
  page: Page,
  sessionId: string,
  initialStatus: KillableStatus,
): Promise<void> {
  let killed = false;
  await page.route(
    new RegExp(`/api/v1/sessions/${sessionId}($|\\?)`),
    async (route) => {
      const method = route.request().method();
      if (method === "DELETE") {
        killed = true;
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({ session_id: sessionId, status: "failed" }),
        });
        return;
      }
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          session_id: sessionId,
          version: 1,
          timestamp: "2026-03-15T10:30:00.000Z",
          session: {
            mode: { type: "headless" },
            agent_config: { system_prompt: "do the thing" },
            mount_spec: { working_dir: "repo", mounts: [] },
            creator: "dev-user",
            status: killed ? "failed" : initialStatus,
            spawned_from: "i-seed00002",
            creation_time: "2026-03-15T10:30:00.000Z",
            start_time: null,
            end_time: null,
          },
        }),
      });
    },
  );
}

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

    // Kill Session button should disappear once the optimistic status update
    // flips the session out of "running" state.
    await expect(killButton).not.toBeVisible();
  });

  test("can kill a created session @sessions:kill", async ({
    authenticatedPage: page,
  }) => {
    const sessionId = "t-created01";
    await mockPreRunningSession(page, sessionId, "created");

    await page.goto(`/issues/i-seed00002/sessions/${sessionId}/logs`);

    const killButton = page.getByRole("button", { name: "Kill Session" });
    await expect(killButton).toBeVisible();

    await killButton.click();
    const modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();
    await modal.getByRole("button", { name: "Kill Session" }).click();

    await expect(modal).not.toBeVisible();
    await expect(page.getByText(/Session killed successfully/)).toBeVisible();
    // Optimistic update flips status to "failed" — button must disappear.
    await expect(killButton).not.toBeVisible();
  });

  test("can kill a pending session @sessions:kill", async ({
    authenticatedPage: page,
  }) => {
    const sessionId = "t-pending01";
    await mockPreRunningSession(page, sessionId, "pending");

    await page.goto(`/issues/i-seed00002/sessions/${sessionId}/logs`);

    const killButton = page.getByRole("button", { name: "Kill Session" });
    await expect(killButton).toBeVisible();

    await killButton.click();
    const modal = page.getByRole("dialog");
    await expect(modal).toBeVisible();
    await modal.getByRole("button", { name: "Kill Session" }).click();

    await expect(modal).not.toBeVisible();
    await expect(page.getByText(/Session killed successfully/)).toBeVisible();
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
