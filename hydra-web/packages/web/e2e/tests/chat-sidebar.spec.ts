import { test, expect } from "../fixtures/auth";

const conversationsFixture = [
  {
    conversation_id: "c-recent-a",
    title: "Recent A",
    agent_name: null,
    status: "active",
    event_count: 3,
    last_event_preview: null,
    creator: "alice",
    created_at: "2026-05-13T10:00:00Z",
    updated_at: "2026-05-13T18:00:00Z",
  },
  {
    conversation_id: "c-recent-b",
    title: "Recent B",
    agent_name: null,
    status: "idle",
    event_count: 1,
    last_event_preview: null,
    creator: "alice",
    created_at: "2026-05-12T09:00:00Z",
    updated_at: "2026-05-12T09:30:00Z",
  },
  {
    conversation_id: "c-oldest",
    title: "Oldest",
    agent_name: null,
    status: "idle",
    event_count: 0,
    last_event_preview: null,
    creator: "alice",
    created_at: "2026-05-01T00:00:00Z",
    updated_at: "2026-05-01T00:00:00Z",
  },
];

test.describe("Sidebar Chats section @chat:sidebar", () => {
  test("clicking a chat row navigates to /chat/<id> @chat:sidebar", async ({
    authenticatedPage: page,
  }) => {
    await page.route(/\/api\/v1\/conversations(\?|$)/, (route) => {
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(conversationsFixture),
      });
    });

    // The sidebar already rendered with empty data during login. Reload so the
    // conversations query refires and picks up the route mock above.
    await page.reload();

    const topRow = page.getByTestId("sidebar-chat-row-c-recent-a");
    await expect(topRow).toBeVisible();
    await expect(topRow).toHaveText("Recent A");

    await topRow.click();
    await expect(page).toHaveURL(/\/chat\/c-recent-a$/);
  });
});
