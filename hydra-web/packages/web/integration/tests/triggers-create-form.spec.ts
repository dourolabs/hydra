import { test, expect } from "../fixtures/auth";

// Status options inside the picker are derived from the project the user
// chose, not a hardcoded list. The five scenarios below exercise the
// project ↔ status picker lifecycle introduced in [[p-dzovcovl]] and gate
// the wire shape: a create_issue action's `project_id` + `status` are
// both required, the form must not submit until they're set, and the
// values the user picked must round-trip into the POST body.
test.describe("Trigger create form @triggers:create-form", () => {
  test("status picker is disabled until a project is picked @triggers:create-form", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/triggers");
    await page.getByRole("button", { name: "Add trigger" }).click();
    const modal = page.getByRole("dialog", { name: "Add Trigger" });
    await expect(modal).toBeVisible();

    const project = modal.getByLabel("Project");
    const status = modal.getByLabel("Status");

    await expect(status).toBeDisabled();

    // Wait for project options to load before selecting — useProjects
    // resolves after the modal mounts.
    await expect(project.locator("option[value='j-defaul']")).toBeAttached();
    await project.selectOption("j-defaul");

    await expect(status).toBeEnabled();
  });

  test("picking a project lists that project's statuses @triggers:create-form", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/triggers");
    await page.getByRole("button", { name: "Add trigger" }).click();
    const modal = page.getByRole("dialog", { name: "Add Trigger" });
    await expect(modal).toBeVisible();

    const project = modal.getByLabel("Project");
    const status = modal.getByLabel("Status");

    await expect(project.locator("option[value='j-engv2']")).toBeAttached();
    await project.selectOption("j-engv2");

    // engineering-v2 declares six statuses; each must surface as an option.
    for (const key of [
      "inbox",
      "backlog",
      "pending",
      "in-development",
      "in-review",
      "pending-release",
    ]) {
      await expect(status.locator(`option[value="${key}"]`)).toHaveCount(1);
    }

    // Default-project keys must NOT leak through — the old hardcoded list
    // would have shown them.
    await expect(status.locator("option[value='open']")).toHaveCount(0);
    await expect(status.locator("option[value='in-progress']")).toHaveCount(0);
  });

  test("changing the project clears the previously-selected status @triggers:create-form", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/triggers");
    await page.getByRole("button", { name: "Add trigger" }).click();
    const modal = page.getByRole("dialog", { name: "Add Trigger" });
    await expect(modal).toBeVisible();

    const project = modal.getByLabel("Project");
    const status = modal.getByLabel("Status");

    await expect(project.locator("option[value='j-defaul']")).toBeAttached();
    await project.selectOption("j-defaul");
    await expect(status.locator("option[value='open']")).toHaveCount(1);
    await status.selectOption("open");
    await expect(status).toHaveValue("open");

    // Switching projects must clear the previously-selected key and re-derive
    // the options against the new project's status list.
    await project.selectOption("j-engv2");
    await expect(status).toHaveValue("");
    await expect(status.locator("option[value='backlog']")).toHaveCount(1);
    await expect(status.locator("option[value='open']")).toHaveCount(0);
  });

  test("submit stays disabled until both project and status are picked @triggers:create-form", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/triggers");
    await page.getByRole("button", { name: "Add trigger" }).click();
    const modal = page.getByRole("dialog", { name: "Add Trigger" });
    await expect(modal).toBeVisible();

    // Fill the other required fields so only project + status gate the button.
    await modal.getByLabel("Cron expression").fill("0 9 * * 1-5");
    await modal
      .getByPlaceholder("Daily standup {{scheduled_at}}")
      .fill("Standup");
    await modal
      .getByPlaceholder("What did the team ship yesterday?")
      .fill("Sync notes");

    const submit = modal.getByRole("button", { name: "Add Trigger" });
    await expect(submit).toBeDisabled();

    const project = modal.getByLabel("Project");
    await expect(project.locator("option[value='j-defaul']")).toBeAttached();
    await project.selectOption("j-defaul");

    // Project set, status still empty — must stay disabled.
    await expect(submit).toBeDisabled();

    const status = modal.getByLabel("Status");
    await expect(status.locator("option[value='in-progress']")).toHaveCount(1);
    await status.selectOption("in-progress");

    await expect(submit).toBeEnabled();
  });

  test("submitting persists the chosen project_id + status in the create_issue action @triggers:create-form", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/triggers");
    await page.getByRole("button", { name: "Add trigger" }).click();
    const modal = page.getByRole("dialog", { name: "Add Trigger" });
    await expect(modal).toBeVisible();

    await modal.getByLabel("Cron expression").fill("0 9 * * 1-5");
    await modal
      .getByPlaceholder("Daily standup {{scheduled_at}}")
      .fill("Standup");
    await modal
      .getByPlaceholder("What did the team ship yesterday?")
      .fill("Sync notes");

    const project = modal.getByLabel("Project");
    await expect(project.locator("option[value='j-engv2']")).toBeAttached();
    await project.selectOption("j-engv2");

    const status = modal.getByLabel("Status");
    await expect(status.locator("option[value='backlog']")).toHaveCount(1);
    await status.selectOption("backlog");

    const [request] = await Promise.all([
      page.waitForRequest(
        (req) =>
          req.method() === "POST" && req.url().endsWith("/v1/triggers"),
      ),
      modal.getByRole("button", { name: "Add Trigger" }).click(),
    ]);

    const body = JSON.parse(request.postData() ?? "{}");
    expect(body.actions).toHaveLength(1);
    expect(body.actions[0].type).toBe("create_issue");
    expect(body.actions[0].project_id).toBe("j-engv2");
    expect(body.actions[0].status).toBe("backlog");

    // Modal closes on success and a toast confirms creation.
    await expect(modal).not.toBeVisible();
    await expect(page.getByText(/Trigger .+ created/)).toBeVisible();
  });
});
