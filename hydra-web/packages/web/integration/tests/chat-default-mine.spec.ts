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
  test("auto-seeds a creator chip on first visit; removing it widens to all chats @chat:default-mine", async ({
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
        body: JSON.stringify({ conversations: body }),
      });
    });

    await page.goto("/chat");
    await expect(page.getByTestId("chats-list")).toBeVisible();

    // The FilterBar's creator chip is auto-seeded for the logged-in user and
    // the URL reflects it.
    await expect(page.getByTestId("filter-chip-creator")).toBeVisible();
    await expect(page).toHaveURL(/[?&]creator=users(?:%2F|\/)dev-user\b/);

    // At least one of the conversations requests was made with creator=<me>.
    expect(requestedCreators).toContain(ME);

    // Only my chats are visible; the foreign-creator chat is filtered out
    // server-side.
    await expect(page.getByTestId("chats-list-row-c-mine-1")).toBeVisible();
    await expect(page.getByTestId("chats-list-row-c-mine-2")).toBeVisible();
    await expect(page.getByTestId("chats-list-row-c-other")).toHaveCount(0);

    // Remove the chip — the URL loses ?creator= and the foreign chat appears.
    await page
      .getByTestId("filter-chip-creator")
      .getByRole("button", { name: /remove creator filter/i })
      .click();
    await expect(page).not.toHaveURL(/[?&]creator=/);
    await expect(page.getByTestId("filter-chip-creator")).toHaveCount(0);
    await expect(page.getByTestId("chats-list-row-c-other")).toBeVisible();
  });

  test("legacy ?scope=mine and ?scope=all URLs redirect to the FilterBar equivalent @chat:default-mine", async ({
    authenticatedPage: page,
  }) => {
    await page.route(/\/api\/v1\/conversations(\?|$)/, (route) => {
      const url = new URL(route.request().url());
      const creator = url.searchParams.get("creator");
      const body = filterByCreator(creator);
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ conversations: body }),
      });
    });

    await page.goto("/chat?scope=mine");
    // The legacy param is rewritten to the explicit `?creator=` on first paint.
    await expect(page).toHaveURL(/[?&]creator=users(?:%2F|\/)dev-user\b/);
    await expect(page).not.toHaveURL(/[?&]scope=/);
    await expect(page.getByTestId("filter-chip-creator")).toBeVisible();

    await page.goto("/chat?scope=all");
    // `?scope=all` strips to "no filter" — no creator chip, no scope param.
    await expect(page).not.toHaveURL(/[?&]scope=/);
    await expect(page).not.toHaveURL(/[?&]creator=/);
    await expect(page.getByTestId("filter-chip-creator")).toHaveCount(0);
    await expect(page.getByTestId("chats-list-row-c-other")).toBeVisible();
  });
});
