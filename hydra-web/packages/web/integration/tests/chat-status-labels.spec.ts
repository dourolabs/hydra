import { test, expect } from "../fixtures/auth";

const fixtureConversations = [
  {
    conversation_id: "c-fixture-active",
    title: "Active conversation",
    agent_name: null,
    status: "active",
    event_count: 3,
    last_event_preview: null,
    creator: "alice",
    created_at: "2026-05-19T01:00:00Z",
    updated_at: "2026-05-19T03:00:00Z",
  },
  {
    conversation_id: "c-fixture-idle",
    title: "Idle conversation",
    agent_name: null,
    status: "idle",
    event_count: 2,
    last_event_preview: null,
    creator: "alice",
    created_at: "2026-05-19T00:30:00Z",
    updated_at: "2026-05-19T02:30:00Z",
  },
  {
    conversation_id: "c-fixture-closed",
    title: "Closed conversation",
    agent_name: null,
    status: "closed",
    event_count: 4,
    last_event_preview: null,
    creator: "alice",
    created_at: "2026-05-19T00:00:00Z",
    updated_at: "2026-05-19T02:00:00Z",
  },
];

test.describe("Chat list status labels @chat:list", () => {
  test("renders literal Active / Idle / Closed status badges @chat:list", async ({
    authenticatedPage: page,
  }) => {
    await page.route(/\/api\/v1\/conversations(\?|$)/, (route) => {
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ conversations: fixtureConversations }),
      });
    });
    await page.goto("/chat");
    await expect(page.getByTestId("chats-list")).toBeVisible();

    const activeRow = page.getByTestId("chats-list-row-c-fixture-active");
    const idleRow = page.getByTestId("chats-list-row-c-fixture-idle");
    const closedRow = page.getByTestId("chats-list-row-c-fixture-closed");

    await expect(activeRow).toContainText("Active");
    await expect(idleRow).toContainText("Idle");
    await expect(closedRow).toContainText("Closed");
  });
});
