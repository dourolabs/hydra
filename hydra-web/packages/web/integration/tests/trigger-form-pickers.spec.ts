import { test, expect } from "../fixtures/auth";

// Two seeded projects with disjoint status sets (see
// packages/mock-server/fixtures/seed.json).
const DEFAULT_PROJECT_ID = "j-defaul";
const DEFAULT_STATUS_KEY = "in-progress";
const DEFAULT_STATUS_LABEL = "In progress";

const ENG_PROJECT_ID = "j-engv2";
const ENG_STATUS_KEYS = [
  "inbox",
  "backlog",
  "pending",
  "in-development",
  "in-review",
  "pending-release",
];

async function openTriggerCreateModal(page: import("@playwright/test").Page) {
  await page.goto("/triggers");
  await expect(page.getByRole("heading", { name: "Triggers" })).toBeVisible();
  await page.getByRole("button", { name: "Add trigger" }).click();
  const modal = page.getByRole("dialog");
  await expect(modal).toBeVisible();
  return modal;
}

test.describe("Trigger form project/status picker @triggers:project-status-picker-lifecycle", () => {
  test("status picker is disabled until a project is picked @triggers:project-status-picker-lifecycle", async ({
    authenticatedPage: page,
  }) => {
    const modal = await openTriggerCreateModal(page);

    const projectSelect = modal.getByLabel("Project");
    const statusSelect = modal.getByLabel("Status");

    await expect(projectSelect).toBeVisible();
    await expect(projectSelect).toHaveValue("");
    // With no project picked the status picker is disabled and the
    // placeholder option directs the user to pick a project first.
    await expect(statusSelect).toBeDisabled();
    await expect(statusSelect).toHaveValue("");
    await expect(
      statusSelect.locator("option", { hasText: "Pick a project first" }),
    ).toHaveCount(1);

    // Picking a project enables the status picker.
    await projectSelect.selectOption(DEFAULT_PROJECT_ID);
    await expect(statusSelect).toBeEnabled();
    await expect(
      statusSelect.locator("option", { hasText: "Select a status…" }),
    ).toHaveCount(1);
  });

  test("switching projects clears the selected status @triggers:project-status-picker-lifecycle", async ({
    authenticatedPage: page,
  }) => {
    const modal = await openTriggerCreateModal(page);

    const projectSelect = modal.getByLabel("Project");
    const statusSelect = modal.getByLabel("Status");

    // Pick project A and one of its statuses.
    await projectSelect.selectOption(DEFAULT_PROJECT_ID);
    // Wait for project A's status options to load before selecting.
    await expect(
      statusSelect.locator(`option[value="${DEFAULT_STATUS_KEY}"]`),
    ).toHaveCount(1);
    await statusSelect.selectOption(DEFAULT_STATUS_KEY);
    await expect(statusSelect).toHaveValue(DEFAULT_STATUS_KEY);

    // Switch to project B. The status is cleared back to the placeholder.
    await projectSelect.selectOption(ENG_PROJECT_ID);
    await expect(statusSelect).toHaveValue("");
  });

  test("switching projects re-derives status options from the new project's status list @triggers:project-status-picker-lifecycle", async ({
    authenticatedPage: page,
  }) => {
    // Hold the `/v1/projects/j-engv2/statuses` request so we can inspect the
    // intermediate state where the new project is picked but its statuses
    // have not yet arrived.
    let releaseEngStatuses: (() => void) | null = null;
    const engStatusesHeld = new Promise<void>((resolve) => {
      releaseEngStatuses = resolve;
    });
    let engStatusesCalls = 0;
    await page.route(
      `**/api/v1/projects/${ENG_PROJECT_ID}/statuses`,
      async (route) => {
        engStatusesCalls += 1;
        if (engStatusesCalls === 1) {
          await engStatusesHeld;
        }
        await route.continue();
      },
    );

    const modal = await openTriggerCreateModal(page);

    const projectSelect = modal.getByLabel("Project");
    const statusSelect = modal.getByLabel("Status");

    // Land on project A first so the picker has known content.
    await projectSelect.selectOption(DEFAULT_PROJECT_ID);
    await expect(
      statusSelect.locator(`option[value="${DEFAULT_STATUS_KEY}"]`),
    ).toHaveCount(1);

    // Now swap to project B. The intercept holds B's status fetch, so the
    // picker should not yet show any of B's statuses; project A's options
    // must also disappear (the form derives options off the active project).
    await projectSelect.selectOption(ENG_PROJECT_ID);
    await expect(statusSelect).toHaveValue("");
    await expect(
      statusSelect.locator(`option[value="${DEFAULT_STATUS_KEY}"]`),
    ).toHaveCount(0);
    for (const key of ENG_STATUS_KEYS) {
      await expect(
        statusSelect.locator(`option[value="${key}"]`),
      ).toHaveCount(0);
    }
    // The placeholder reflects that a project is picked but no status yet.
    await expect(
      statusSelect.locator("option", { hasText: "Select a status…" }),
    ).toHaveCount(1);

    // Release the held fetch — project B's statuses populate the picker.
    releaseEngStatuses!();
    for (const key of ENG_STATUS_KEYS) {
      await expect(
        statusSelect.locator(`option[value="${key}"]`),
      ).toHaveCount(1);
    }
    await expect(
      statusSelect.locator(`option[value="${DEFAULT_STATUS_KEY}"]`),
    ).toHaveCount(0);

    // A status from project B is selectable.
    await statusSelect.selectOption("backlog");
    await expect(statusSelect).toHaveValue("backlog");
    // sanity-check: the picker really did pull from project B's label set.
    await expect(
      statusSelect.locator("option", { hasText: DEFAULT_STATUS_LABEL }),
    ).toHaveCount(0);
  });
});
