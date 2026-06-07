import { test, expect } from "../fixtures/auth";

const ME = "dev-user";

const fixtureConversations = [
  {
    conversation_id: "c-active-mine",
    title: "Active chat (mine)",
    agent_name: null,
    status: "active",
    event_count: 2,
    last_event_preview: null,
    creator: ME,
    created_at: "2026-05-13T10:00:00Z",
    updated_at: "2026-05-13T18:00:00Z",
  },
  {
    conversation_id: "c-idle-mine",
    title: "Idle chat (mine)",
    agent_name: null,
    status: "idle",
    event_count: 1,
    last_event_preview: null,
    creator: ME,
    created_at: "2026-05-12T09:00:00Z",
    updated_at: "2026-05-12T09:30:00Z",
  },
  {
    conversation_id: "c-closed-mine",
    title: "Closed chat (mine)",
    agent_name: null,
    status: "closed",
    event_count: 5,
    last_event_preview: null,
    creator: ME,
    created_at: "2026-05-10T09:00:00Z",
    updated_at: "2026-05-10T09:30:00Z",
  },
];

function filterByStatusAndCreator(
  status: string | null,
  creator: string | null,
) {
  return fixtureConversations.filter((c) => {
    if (creator && c.creator !== creator) return false;
    if (status && c.status !== status) return false;
    return true;
  });
}

test.describe("Chats page FilterBar @chat:filter-bar", () => {
  test("picking a Status chip narrows server-side and persists to URL @chat:filter-bar", async ({
    authenticatedPage: page,
  }) => {
    const requestedQueries: Array<{
      status: string | null;
      creator: string | null;
    }> = [];
    await page.route(/\/api\/v1\/conversations(\?|$)/, (route) => {
      const url = new URL(route.request().url());
      const status = url.searchParams.get("status");
      const creator = url.searchParams.get("creator");
      requestedQueries.push({ status, creator });
      const body = filterByStatusAndCreator(status, creator);
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(body),
      });
    });

    await page.goto("/chat");
    // Page mounts with the auto-creator chip in place.
    await expect(page.getByTestId("filter-chip-creator")).toBeVisible();

    // Open the add-filter menu and pick Status.
    const addFilter = page.getByTestId("filter-bar-add");
    await expect(addFilter).toBeVisible();
    await addFilter.click();
    await expect(page.getByTestId("add-filter-menu")).toBeVisible();
    await expect(page.getByTestId("add-filter-status")).toBeVisible();
    await page.getByTestId("add-filter-status").click();

    // The status chip appears and the value picker opens.
    await expect(page.getByTestId("filter-chip-status")).toBeVisible();
    await expect(page.getByTestId("value-picker-status")).toBeVisible();

    // Picking "active" narrows the result + writes `?status=active` to URL.
    await page.getByTestId("value-option-active").click();
    await expect(page).toHaveURL(/[?&]status=active\b/);

    // Only the active row is visible. The other rows were filtered out
    // server-side (we drop them in the route handler when ?status=active).
    await expect(page.getByTestId("chats-list-row-c-active-mine")).toBeVisible();
    await expect(page.getByTestId("chats-list-row-c-idle-mine")).toHaveCount(0);
    await expect(page.getByTestId("chats-list-row-c-closed-mine")).toHaveCount(0);

    // Verify the server was called with both creator=<me> and status=active.
    const matched = requestedQueries.find(
      (q) => q.status === "active" && q.creator === ME,
    );
    expect(matched).toBeTruthy();

    // Remove the creator chip — the URL loses ?creator= but keeps ?status=active.
    await page
      .getByTestId("filter-chip-creator")
      .getByRole("button", { name: /remove creator filter/i })
      .click();
    await expect(page).not.toHaveURL(/[?&]creator=/);
    await expect(page).toHaveURL(/[?&]status=active\b/);
  });
});
