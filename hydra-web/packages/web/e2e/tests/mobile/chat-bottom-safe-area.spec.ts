import { test, expect } from "../../fixtures/auth";
import type { Page } from "@playwright/test";
import type { ConversationEvent } from "@hydra/api";

// Verifies the AppLayout main scroll container reserves room for iOS Safari's
// home-indicator safe area on the chat detail page.
//
// Playwright's Chromium engine resolves env(safe-area-inset-bottom) to 0 — the
// emulator does not inject a non-zero inset. We therefore override the
// `--safe-area-bottom` token (which the fix wraps env() in) at the document
// root and assert that the computed padding-bottom on the AppLayout main
// scales with the override. On origin/main, padding-bottom is a fixed 56px
// regardless of the variable, so this test fails. With the fix in place,
// padding-bottom is calc(56px + var(--safe-area-bottom)) and the override
// drives the computed value up by the simulated inset.

const CONVERSATION_ID = "c-mobile-bottom-safe-area";
const SIMULATED_SAFE_AREA_PX = 34; // iPhone-13-class home indicator height
const BASE_BUFFER_PX = 56;

const conversation = {
  conversation_id: CONVERSATION_ID,
  title: "Safe area conversation",
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

const events: ConversationEvent[] = [
  { type: "user_message", content: "Hello", timestamp: "2026-05-13T10:00:00Z" },
  { type: "assistant_message", content: "Hi there", timestamp: "2026-05-13T10:01:00Z" },
];

async function mockChatRoutes(page: Page) {
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
  await page.route(new RegExp(`/api/v1/conversations/${CONVERSATION_ID}/events$`), (route) => {
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(events),
    });
  });
}

async function injectSimulatedSafeArea(page: Page, px: number) {
  await page.addStyleTag({
    content: `:root { --safe-area-bottom: ${px}px !important; }`,
  });
}

test.describe("Mobile chat bottom safe-area @mobile:chat-bottom-safe-area", () => {
  test.use({ viewport: { width: 390, height: 844 } });

  test("AppLayout main reserves space for env(safe-area-inset-bottom) on chat detail @mobile:chat-bottom-safe-area", async ({
    authenticatedPage: page,
  }) => {
    await mockChatRoutes(page);
    await page.goto(`/chat/${CONVERSATION_ID}`);

    // Wait for the composer to render so layout is settled.
    await expect(page.getByPlaceholder("Type a message…")).toBeVisible();

    // Hard stop: env(safe-area-inset-bottom) is gated behind viewport-fit=cover
    // on iOS Safari. Without the meta opt-in, the CSS fix is a no-op on the
    // platform that needs it most, regardless of how the calc reads in tests.
    const viewportMetaContent = await page
      .locator('meta[name="viewport"]')
      .getAttribute("content");
    expect(viewportMetaContent ?? "").toContain("viewport-fit=cover");

    await injectSimulatedSafeArea(page, SIMULATED_SAFE_AREA_PX);

    // The AppLayout `<main>` is the scroll container whose padding-bottom
    // protects the composer from the home indicator. Assert its computed
    // padding-bottom grows with the simulated safe-area inset.
    const paddingBottom = await page.evaluate(() => {
      const main = document.querySelector("main");
      if (!main) throw new Error("AppLayout <main> not found");
      return parseFloat(window.getComputedStyle(main).paddingBottom);
    });

    expect(paddingBottom).toBeGreaterThanOrEqual(BASE_BUFFER_PX + SIMULATED_SAFE_AREA_PX);

    // User-visible geometry: nothing in the chat pane should poke past the
    // visible viewport, even with the home-indicator inset reserved. This is
    // the assertion the prior tautological padding check missed.
    await page.getByTestId("chat-message-list").scrollIntoViewIfNeeded();
    const chatPaneBox = await page.getByTestId("chat-pane").boundingBox();
    const viewport = page.viewportSize();
    if (!chatPaneBox) throw new Error("chat-pane bounding box not available");
    if (!viewport) throw new Error("viewport size not available");
    expect(chatPaneBox.y + chatPaneBox.height).toBeLessThanOrEqual(viewport.height);
  });
});
