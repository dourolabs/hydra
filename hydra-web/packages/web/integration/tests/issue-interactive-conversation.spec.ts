import { test, expect } from "../fixtures/auth";

// Seeded conversations linked to issues via `spawned_from` (see
// packages/mock-server/fixtures/seed.json):
//   c-seed00008  active  spawned_from: i-seed00001
//   c-seed00009  closed  spawned_from: i-seed00001
//   c-seed00010  idle    spawned_from: i-seed00006

test.describe("Interactive conversation surfacing @issues:interactive-conversation", () => {
  test("issue header shows 'Open Conversation' when an active spawned conversation exists @issues:interactive-conversation", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00001");

    const link = page.getByTestId("issue-open-conversation");
    await expect(link).toBeVisible();
    await expect(link).toHaveText("Open Conversation");
    await expect(link).toHaveAttribute("data-conversation-status", "active");
    await expect(link).toHaveAttribute("href", "/chat/c-seed00008");
  });

  test("issue header shows 'Resume Conversation' for an idle spawned conversation @issues:interactive-conversation", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00006");

    const link = page.getByTestId("issue-open-conversation");
    await expect(link).toBeVisible();
    await expect(link).toHaveText("Resume Conversation");
    await expect(link).toHaveAttribute("data-conversation-status", "idle");
    await expect(link).toHaveAttribute("href", "/chat/c-seed00010");
  });

  test("Related tab lists every conversation linked to the issue, live + closed @issues:interactive-conversation", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00001");

    // Related is the default right-rail tab.
    const liveRow = page.getByTestId("related-rail-row-chat-c-seed00008");
    const closedRow = page.getByTestId("related-rail-row-chat-c-seed00009");
    await expect(liveRow).toBeVisible();
    await expect(closedRow).toBeVisible();
  });

  test("clicking 'Open Conversation' lands in chat with originated-from link @issues:interactive-conversation", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/issues/i-seed00001");

    const link = page.getByTestId("issue-open-conversation");
    await expect(link).toBeVisible();
    await link.click();

    await page.waitForURL(/\/chat\/c-seed00008$/);

    const originated = page.getByTestId("chat-header-originated-from");
    await expect(originated).toBeVisible();
    await expect(originated).toContainText("originated from");
    await expect(originated.getByRole("link", { name: "i-seed00001" })).toHaveAttribute(
      "href",
      "/issues/i-seed00001",
    );
  });

  test("no affordance is shown when no live spawned conversation exists @issues:interactive-conversation", async ({
    authenticatedPage: page,
  }) => {
    // i-seed00002 has no `spawned_from` conversations in the seed.
    await page.goto("/issues/i-seed00002");
    await expect(page.getByTestId("issue-open-conversation")).toHaveCount(0);
  });
});

test.describe("Project editor interactive flag @projects:interactive-status", () => {
  test("editor exposes an Interactive checkbox alongside existing status flags @projects:interactive-status", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/projects");
    await page.getByTestId("projects-list-add").click();
    await expect(page.getByRole("dialog")).toBeVisible();

    // First default status ("open") has interactive: false initially.
    const interactive = page.getByTestId("status-editor-interactive-0");
    await expect(interactive).toBeVisible();
    await expect(interactive).not.toBeChecked();

    await interactive.check();
    await expect(interactive).toBeChecked();
  });
});
