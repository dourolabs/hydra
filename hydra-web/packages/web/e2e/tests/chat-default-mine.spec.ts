import { test, expect } from "../fixtures/auth";

// The mock-server login fixture authenticates as DEV_USERNAME ("dev-user").
const ME = "dev-user";

const fixtureConversations = [
  {
    conversation_id: "c-mine-1",
    title: "Mine A",
    agent_name: null,
    status: "active",
    event_count: 1,
    last_event_preview: null,
    creator: ME,
    created_at: "2026-05-13T10:00:00Z",
    updated_at: "2026-05-13T18:00:00Z",
  },
  {
    conversation_id: "c-mine-2",
    title: "Mine B",
    agent_name: null,
    status: "idle",
    event_count: 1,
    last_event_preview: null,
    creator: ME,
    created_at: "2026-05-12T09:00:00Z",
    updated_at: "2026-05-12T09:30:00Z",
  },
  {
    conversation_id: "c-other",
    title: "Other person's chat",
    agent_name: null,
    status: "active",
    event_count: 2,
    last_event_preview: null,
    creator: "someone-else",
    created_at: "2026-05-11T08:00:00Z",
    updated_at: "2026-05-11T08:30:00Z",
  },
];

function filterByCreator(creator: string | null) {
  if (!creator) return fixtureConversations;
  return fixtureConversations.filter((c) => c.creator === creator);
}

test.describe("Chat list defaults to current user @chat:default-mine", () => {
  test("defaults to Mine, toggles to All, and persists scope in URL @chat:default-mine", async ({
    authenticatedPage: page,
  }) => {
    const requestedCreators: Array<string | null> = [];
    await page.route(/\/api\/v1\/conversations(\?|$)/, (route) => {
      const url = new URL(route.request().url());
      const creator = url.searchParams.get("creator");
      requestedCreators.push(creator);
      const body = filterByCreator(creator);
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(body),
      });
    });

    await page.goto("/chat");
    await expect(page.getByTestId("chats-list")).toBeVisible();

    // The Mine pill is selected by default.
    const mine = page.getByTestId("chats-scope-mine");
    const all = page.getByTestId("chats-scope-all");
    await expect(mine).toHaveAttribute("aria-selected", "true");
    await expect(all).toHaveAttribute("aria-selected", "false");

    // At least one of the conversations requests was made with creator=<me>.
    expect(requestedCreators).toContain(ME);

    // Only my chats are visible; the foreign-creator chat is filtered out.
    await expect(page.getByTestId("chats-list-row-c-mine-1")).toBeVisible();
    await expect(page.getByTestId("chats-list-row-c-mine-2")).toBeVisible();
    await expect(page.getByTestId("chats-list-row-c-other")).toHaveCount(0);

    // Toggle to All — URL gains ?scope=all and the foreign chat appears.
    await all.click();
    await expect(page).toHaveURL(/[?&]scope=all\b/);
    await expect(all).toHaveAttribute("aria-selected", "true");
    await expect(mine).toHaveAttribute("aria-selected", "false");
    await expect(page.getByTestId("chats-list-row-c-other")).toBeVisible();

    // Toggle back to Mine — scope=all is removed from the URL.
    await mine.click();
    await expect(page).not.toHaveURL(/[?&]scope=all\b/);
    await expect(mine).toHaveAttribute("aria-selected", "true");
    await expect(page.getByTestId("chats-list-row-c-other")).toHaveCount(0);
  });
});
