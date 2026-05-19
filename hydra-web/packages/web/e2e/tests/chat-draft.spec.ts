import { test, expect } from "../fixtures/auth";

// The mock-server seed ships two conversations (c-seed00001 active,
// c-seed00002 closed). The sidebar filters closed conversations out, so
// override the LIST query to surface both rows in the sidebar — the per-id
// detail endpoints remain real, so navigating actually loads each chat.
const sidebarConversations = [
  {
    conversation_id: "c-seed00001",
    title: "Welcome to Hydra",
    agent_name: null,
    status: "active",
    event_count: 4,
    last_event_preview: null,
    creator: "dev-user",
    created_at: "2026-05-10T14:00:00.000Z",
    updated_at: "2026-05-10T14:05:00.000Z",
  },
  {
    conversation_id: "c-seed00002",
    title: "Q1 retro notes",
    agent_name: "scribe",
    status: "active",
    event_count: 2,
    last_event_preview: null,
    creator: "alice",
    created_at: "2026-05-08T09:30:00.000Z",
    updated_at: "2026-05-08T10:15:00.000Z",
  },
];

test.describe("Chat composer draft persistence @chat:draft-per-conversation", () => {
  test("textarea shows each conversation's own draft when switching via sidebar SPA navigation @chat:draft-per-conversation", async ({
    authenticatedPage: page,
  }) => {
    await page.route(/\/api\/v1\/conversations(\?|$)/, (route) => {
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(sidebarConversations),
      });
    });
    // The sidebar already issued the conversations query during login with
    // the unmocked response. Reload so it re-fetches against the mock above
    // and the closed seed conversation is surfaced as a clickable row.
    await page.reload();

    const rowOne = page.getByTestId("sidebar-chat-row-c-seed00001");
    const rowTwo = page.getByTestId("sidebar-chat-row-c-seed00002");
    await expect(rowOne).toBeVisible();
    await expect(rowTwo).toBeVisible();

    const textarea = page.getByPlaceholder("Type a message…");

    await rowOne.click();
    await expect(page).toHaveURL(/\/chat\/c-seed00001$/);
    await expect(textarea).toBeVisible();
    await textarea.fill("alpha");
    await expect(textarea).toHaveValue("alpha");

    await rowTwo.click();
    await expect(page).toHaveURL(/\/chat\/c-seed00002$/);
    await expect(textarea).toBeVisible();
    // Second conversation has no stored draft yet — it must NOT inherit the
    // first conversation's "alpha".
    await expect(textarea).toHaveValue("");
    await textarea.fill("beta");
    await expect(textarea).toHaveValue("beta");

    // Capture the textarea DOM node *before* navigating back to c-seed00001.
    // Both conversations are now cached so neither shows the loading state,
    // meaning ChatPage stays mounted across the conversationId change — this
    // is the path where the old "useState + useEffect resync" approach would
    // flash the previous draft. With key={conversationId} on <ChatInput/>,
    // React unmounts and remounts the input so the new draft is the very
    // first value the user sees.
    const beforeNavHandle = await textarea.elementHandle();

    await rowOne.click();
    await expect(page).toHaveURL(/\/chat\/c-seed00001$/);
    await expect(textarea).toHaveValue("alpha");
    // The textarea that held "beta" must no longer be in the DOM — proves
    // ChatInput remounted (the structural fix), not just patched in place
    // via the post-commit effect.
    expect(beforeNavHandle).not.toBeNull();
    expect(await beforeNavHandle!.evaluate((el) => el.isConnected)).toBe(false);

    await rowTwo.click();
    await expect(page).toHaveURL(/\/chat\/c-seed00002$/);
    await expect(textarea).toHaveValue("beta");
  });
});
