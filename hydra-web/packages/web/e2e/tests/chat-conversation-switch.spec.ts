import { test, expect } from "../fixtures/auth";

// Regression test for the chat transcript leak: when navigating directly
// between two existing conversations (via the sidebar or URL), the second
// conversation's transcript must not transiently or persistently include
// messages from the first one.

// The sidebar's "My chats" section only shows the logged-in user's open
// chats. The seed (under DEV_USERNAME=dev-user) only has one non-closed
// dev-user conversation, so we override the conversations list endpoint to
// also surface c-seed00002 (alice's, but pretending it belongs to dev-user)
// in the sidebar — this is the only way to test sidebar→sidebar soft
// navigation between two conversations without going through a different
// route (which would unmount the ChatPage component and mask the bug).
const dualSidebarConversations = [
  {
    conversation_id: "c-seed00001",
    title: "Welcome to Hydra",
    agent_name: null,
    status: "active",
    creator: "dev-user",
    event_count: 4,
    last_event_preview: null,
    created_at: "2026-05-10T14:00:00.000Z",
    updated_at: "2026-05-10T14:05:00.000Z",
  },
  {
    conversation_id: "c-seed00002",
    title: "Q1 retro notes",
    agent_name: "scribe",
    status: "active",
    creator: "dev-user",
    event_count: 2,
    last_event_preview: null,
    created_at: "2026-05-08T09:30:00.000Z",
    updated_at: "2026-05-08T10:15:00.000Z",
  },
];

test.describe("Chat conversation switch @chat:conversation-switch", () => {
  test("transcript for the new conversation does not include the previous conversation's messages @chat:conversation-switch", async ({
    authenticatedPage: page,
  }) => {
    await page.route(/\/api\/v1\/conversations(\?|$)/, (route) => {
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(dualSidebarConversations),
      });
    });
    await page.goto("/chat/c-seed00001");

    const list = page.getByTestId("chat-message-list");
    await expect(list).toBeVisible();
    await expect(list).toContainText("Show me what's going on");
    await expect(list).toContainText("The Platform v2.0 migration is in progress");

    // Soft-navigate via the sidebar so react-router stays on the same route
    // (`/chat/:conversationId`) and reuses the ChatPage component instance —
    // this is the path the bug surfaces under.
    await page.getByTestId("sidebar-chat-row-c-seed00002").click();
    await page.waitForURL(/\/chat\/c-seed00002$/);

    await expect(list).toContainText("Summarize what shipped this quarter");
    await expect(list).toContainText("Q1 highlights");

    // Crucially, c-seed00001's messages must not have leaked into the
    // c-seed00002 transcript.
    await expect(list).not.toContainText("Show me what's going on");
    await expect(list).not.toContainText("The Platform v2.0 migration is in progress");
  });

  test("optimistic message sent in one conversation does not leak into the next conversation @chat:conversation-switch", async ({
    authenticatedPage: page,
  }) => {
    await page.route(/\/api\/v1\/conversations(\?|$)/, (route) => {
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(dualSidebarConversations),
      });
    });
    await page.goto("/chat/c-seed00001");

    const list = page.getByTestId("chat-message-list");
    await expect(list).toContainText("Show me what's going on");

    // Slow the send response so the optimistic event has time to be on screen,
    // and so we can navigate away before reconciliation lands.
    let releaseSend: () => void;
    const sendBlocked = new Promise<void>((resolve) => {
      releaseSend = resolve;
    });
    await page.route(/\/api\/v1\/conversations\/c-seed00001\/messages$/, async (route) => {
      await sendBlocked;
      await route.continue();
    });

    const composer = page.getByPlaceholder("Type a message…");
    await composer.fill("c-one-pending");
    await page.getByRole("button", { name: "Send" }).click();

    // Optimistic event is rendered in c-seed00001.
    await expect(list).toContainText("c-one-pending");

    // Soft-navigate to c-seed00002 via the sidebar before the send mutation
    // settles — the ChatPage component instance is reused, so any
    // ChatPage-local state (like the optimistic event buffer) is at risk of
    // leaking.
    await page.getByTestId("sidebar-chat-row-c-seed00002").click();
    await page.waitForURL(/\/chat\/c-seed00002$/);
    await expect(list).toContainText("Summarize what shipped this quarter");

    // c-seed00001's optimistic message must not leak into c-seed00002.
    await expect(list).not.toContainText("c-one-pending");

    releaseSend!();
  });
});
