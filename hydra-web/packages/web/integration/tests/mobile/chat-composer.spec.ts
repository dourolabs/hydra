import { test, expect } from "../../fixtures/auth";
import type { SessionEvent } from "@hydra/api";

const CONVERSATION_ID = "c-mobile-composer";
const SESSION_ID = "t-mobile-composer";

const conversation = {
  conversation_id: CONVERSATION_ID,
  title: "Composer conversation",
  agent_name: "TestAgent",
  status: "active",
  creator: "alice",
  created_at: "2026-05-13T10:00:00Z",
  updated_at: "2026-05-13T10:30:00Z",
};

const conversationSummary = {
  conversation_id: CONVERSATION_ID,
  title: conversation.title,
  agent_name: conversation.agent_name,
  status: conversation.status,
  event_count: 2,
  last_event_preview: "Hi there",
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

const sessionEvents: SessionEvent[] = [
  { type: "user_message", content: "Hello", timestamp: "2026-05-13T10:00:00Z" },
  { type: "assistant_message", content: "Hi there", timestamp: "2026-05-13T10:01:00Z" },
];

async function mockChatRoutes(page: import("@playwright/test").Page) {
  await page.route(/\/api\/v1\/conversations(\?|$)/, (route) => {
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify([conversationSummary]),
    });
  });
  await page.route(new RegExp(`/api/v1/conversations/${CONVERSATION_ID}$`), (route) => {
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(conversation),
    });
  });
  // SessionEvent per-session fan-out: list sessions for the conversation,
  // then fetch each session's event log.
  await page.route(new RegExp(`/api/v1/sessions/${SESSION_ID}/events($|\\?)`), (route) => {
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(sessionEvents),
    });
  });
  await page.route(/\/api\/v1\/sessions(\?|$)/, (route) => {
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ sessions: [sessionSummary] }),
    });
  });
}

// Parse `rgb(...)` / `rgba(...)` / `oklch(...)` etc. via a hidden swatch.
// Browsers normalize getComputedStyle().backgroundColor to rgb()/rgba(),
// so a string comparison is reliable for equality checks.
async function getComputedBg(
  page: import("@playwright/test").Page,
  selector: string,
): Promise<string> {
  return await page.evaluate((sel) => {
    const el = document.querySelector(sel);
    if (!el) throw new Error(`Element not found: ${sel}`);
    return window.getComputedStyle(el as Element).backgroundColor;
  }, selector);
}

test.describe("Mobile chat composer @mobile:chat-composer", () => {
  test.use({ viewport: { width: 390, height: 844 } });

  test("textarea uses ≥16px font-size to prevent iOS zoom and is visually distinct from page bg @mobile:chat-composer", async ({
    authenticatedPage: page,
  }) => {
    await mockChatRoutes(page);
    await page.goto(`/chat/${CONVERSATION_ID}`);

    const textarea = page.getByPlaceholder("Type a message…");
    await expect(textarea).toBeVisible();

    // Acceptance criterion 2: computed font-size at ≤768px viewports is ≥16px.
    // This is the iOS-Safari workaround — Safari zooms the viewport on focus
    // whenever the focused field's font-size is <16px.
    const fontSizePx = await textarea.evaluate((el) =>
      parseFloat(window.getComputedStyle(el).fontSize),
    );
    expect(fontSizePx).toBeGreaterThanOrEqual(16);

    // Acceptance criterion 3: the composer textarea is visually distinguishable
    // from the page background. Compare the textarea's computed
    // background-color against the page body background; they must differ.
    const textareaBg = await textarea.evaluate((el) => window.getComputedStyle(el).backgroundColor);
    const bodyBg = await getComputedBg(page, "body");
    expect(textareaBg).not.toBe(bodyBg);
    // Sanity: the textarea must have an actual paint (not transparent).
    expect(textareaBg).not.toBe("rgba(0, 0, 0, 0)");
    expect(textareaBg).not.toBe("transparent");
  });

  test("light theme: composer textarea is visually distinct from page bg @mobile:chat-composer", async ({
    authenticatedPage: page,
  }) => {
    await mockChatRoutes(page);
    await page.addInitScript(() => {
      document.documentElement.setAttribute("data-theme", "light");
    });
    await page.goto(`/chat/${CONVERSATION_ID}`);

    const textarea = page.getByPlaceholder("Type a message…");
    await expect(textarea).toBeVisible();

    const textareaBg = await textarea.evaluate((el) => window.getComputedStyle(el).backgroundColor);
    const bodyBg = await getComputedBg(page, "body");
    expect(textareaBg).not.toBe(bodyBg);
  });
});
