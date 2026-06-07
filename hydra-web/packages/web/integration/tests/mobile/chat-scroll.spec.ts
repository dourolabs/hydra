import { test, expect } from "../../fixtures/auth";
import type { SessionEvent } from "@hydra/api";

const CONVERSATION_ID = "c-mobile-scroll";
const SESSION_ID = "t-mobile-scroll";

const conversation = {
  conversation_id: CONVERSATION_ID,
  title: "Long conversation",
  agent_name: "TestAgent",
  status: "active",
  creator: "alice",
  created_at: "2026-05-13T10:00:00Z",
  updated_at: "2026-05-13T18:00:00Z",
};

const conversationSummary = {
  conversation_id: CONVERSATION_ID,
  title: conversation.title,
  agent_name: conversation.agent_name,
  status: conversation.status,
  event_count: 30,
  last_event_preview: "Message 30",
  creator: conversation.creator,
  created_at: conversation.created_at,
  updated_at: conversation.updated_at,
};

const sessionSummary = {
  session_id: SESSION_ID,
  version: 1,
  timestamp: "2026-05-13T10:00:00Z",
  session: {
    prompt: "",
    creator: "alice",
    status: "running",
    creation_time: "2026-05-13T10:00:00Z",
  },
};

function buildEvents(count: number): SessionEvent[] {
  const out: SessionEvent[] = [];
  for (let i = 1; i <= count; i++) {
    const ts = new Date(Date.UTC(2026, 4, 13, 10, i)).toISOString();
    if (i % 2 === 1) {
      out.push({ type: "user_message", content: `Message ${i} from user`, timestamp: ts });
    } else {
      out.push({
        type: "assistant_message",
        content: `Message ${i} from agent`,
        timestamp: ts,
      });
    }
  }
  return out;
}

test.use({ viewport: { width: 375, height: 700 } });

test.describe("Mobile chat scroll @mobile:chat-scroll", () => {
  test("chat header stays visible and message list owns scroll @mobile:chat-scroll", async ({
    authenticatedPage: page,
  }) => {
    await page.route(/\/api\/v1\/conversations(\?|$)/, (route) => {
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify([conversationSummary]),
      });
    });
    await page.route(
      new RegExp(`/api/v1/conversations/${CONVERSATION_ID}$`),
      (route) => {
        route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify(conversation),
        });
      },
    );
    await page.route(
      new RegExp(`/api/v1/sessions/${SESSION_ID}/events($|\\?)`),
      (route) => {
        route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify(buildEvents(30)),
        });
      },
    );
    await page.route(/\/api\/v1\/sessions(\?|$)/, (route) => {
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ sessions: [sessionSummary] }),
      });
    });

    await page.goto(`/chat/${CONVERSATION_ID}`);

    const title = page.getByRole("heading", { name: "Long conversation" });
    await expect(title).toBeVisible();
    // After messages load, the ChatHeader title must remain in the viewport —
    // the page must NOT have scrolled itself (or any ancestor) to the bottom.
    await expect(title).toBeInViewport();

    const messageList = page.getByTestId("chat-message-list");
    await expect(messageList).toBeVisible();
    // Wait for the auto-scroll-to-bottom effect to settle so we can then
    // confirm a manual scroll-up is not snapped back.
    await expect
      .poll(async () =>
        messageList.evaluate(
          (el) => el.scrollHeight > el.clientHeight && el.scrollTop > 0,
        ),
      )
      .toBe(true);

    const scrollState = await messageList.evaluate((el) => {
      const maxScroll = el.scrollHeight - el.clientHeight;
      el.scrollTop = Math.max(0, maxScroll / 2);
      return {
        scrollTop: el.scrollTop,
        scrollHeight: el.scrollHeight,
        clientHeight: el.clientHeight,
      };
    });

    // Container is scrollable AND not pinned to the bottom — user can read
    // older messages without being snapped back.
    expect(scrollState.scrollHeight).toBeGreaterThan(scrollState.clientHeight);
    expect(scrollState.scrollTop).toBeLessThan(
      scrollState.scrollHeight - scrollState.clientHeight,
    );

    // ChatHeader stays visible after the user scrolled the message list.
    await expect(title).toBeInViewport();

    // The page body must NOT be the scrollable surface — scrolling the
    // message list must not have moved any page-level scroll position.
    const documentScrollTop = await page.evaluate(
      () => document.scrollingElement?.scrollTop ?? window.scrollY,
    );
    expect(documentScrollTop).toBe(0);
  });
});
