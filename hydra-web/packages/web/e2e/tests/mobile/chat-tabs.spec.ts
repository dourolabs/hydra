import { test, expect } from "../../fixtures/auth";
import type { ConversationEvent } from "@hydra/api";

const CONVERSATION_ID = "c-mobile-scroll";

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

function buildEvents(count: number): ConversationEvent[] {
  const out: ConversationEvent[] = [];
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

// On mobile the sidebar drawer is hidden=false by default, which means it
// auto-slides in and intercepts pointer events on tabs along the top of the
// chat page. Persist "hidden" before navigation so the drawer stays closed.
async function setSidebarHidden(page: import("@playwright/test").Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem("hydra-sidebar-hidden", "1");
  });
}

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
  await page.route(new RegExp(`/api/v1/conversations/${CONVERSATION_ID}/events$`), (route) => {
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(buildEvents(30)),
    });
  });
  // ChatRelatedTab pulls relations; return an empty list so the tab renders
  // its empty-state copy without firing additional sub-resource fetches.
  await page.route(/\/api\/v1\/relations(\?|$)/, (route) => {
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ relations: [] }),
    });
  });
}

test.describe("Mobile chat tabs @mobile:chat-tabs", () => {
  test.describe("at 375x700 viewport", () => {
    test.use({ viewport: { width: 375, height: 700 } });

    test("renders three tabs with Chat as default, panes toggle without page scroll @mobile:chat-tabs", async ({
      authenticatedPage: page,
    }) => {
      await setSidebarHidden(page);
      await mockChatRoutes(page);
      await page.goto(`/chat/${CONVERSATION_ID}`);

      const chatTab = page.getByTestId("chat-mobile-tab-chat");
      const relatedTab = page.getByTestId("chat-mobile-tab-related");
      const settingsTab = page.getByTestId("chat-mobile-tab-settings");

      await expect(chatTab).toBeVisible();
      await expect(relatedTab).toBeVisible();
      await expect(settingsTab).toBeVisible();
      await expect(chatTab).toHaveAttribute("aria-selected", "true");
      await expect(relatedTab).toHaveAttribute("aria-selected", "false");
      await expect(settingsTab).toHaveAttribute("aria-selected", "false");

      const title = page.getByRole("heading", { name: "Long conversation" });
      await expect(title).toBeVisible();
      await expect(title).toBeInViewport();

      const messageList = page.getByTestId("chat-message-list");
      await expect(messageList).toBeVisible();

      // p-ufesgw scroll-lock invariant — the page body must NOT have scrolled.
      const initialDocScroll = await page.evaluate(
        () => document.scrollingElement?.scrollTop ?? window.scrollY,
      );
      expect(initialDocScroll).toBe(0);

      // Switch to Related — message list hides, related content visible.
      await relatedTab.click();
      await expect(relatedTab).toHaveAttribute("aria-selected", "true");
      await expect(messageList).toBeHidden();
      await expect(page.getByText("No issues referenced by this chat yet.")).toBeVisible();

      // Switch to Settings — related hides, settings visible.
      await settingsTab.click();
      await expect(settingsTab).toHaveAttribute("aria-selected", "true");
      await expect(page.getByText("No issues referenced by this chat yet.")).toBeHidden();
      await expect(page.getByText("Conversation ID")).toBeVisible();
      await expect(page.getByText(CONVERSATION_ID).first()).toBeVisible();

      // Switching tabs does not introduce page-level scroll either.
      const afterSwitchDocScroll = await page.evaluate(
        () => document.scrollingElement?.scrollTop ?? window.scrollY,
      );
      expect(afterSwitchDocScroll).toBe(0);
    });

    test("returning to Chat preserves message-list scroll position @mobile:chat-tabs", async ({
      authenticatedPage: page,
    }) => {
      await setSidebarHidden(page);
      await mockChatRoutes(page);
      await page.goto(`/chat/${CONVERSATION_ID}`);

      const messageList = page.getByTestId("chat-message-list");
      await expect(messageList).toBeVisible();

      // Wait for the auto-scroll-to-bottom smooth animation to SETTLE (not just
      // start). ChatMessageList.tsx:70 uses `scrollTo({ behavior: "smooth" })`,
      // so `scrollTop > 0` flips true the moment the animation begins. If we
      // proceed at that point, setting `scrollTop = target` below races the
      // in-flight smooth scroll and produces drift. Waiting until scrollTop has
      // reached the bottom (within 1px) guarantees the animation has finished.
      await expect
        .poll(async () =>
          messageList.evaluate(
            (el) =>
              el.scrollHeight > el.clientHeight &&
              el.scrollTop >= el.scrollHeight - el.clientHeight - 1,
          ),
        )
        .toBe(true);

      // Scroll the message list halfway up.
      const targetScroll = await messageList.evaluate((el) => {
        const target = Math.max(0, (el.scrollHeight - el.clientHeight) / 2);
        el.scrollTop = target;
        return el.scrollTop;
      });
      expect(targetScroll).toBeGreaterThan(0);

      // Switch away and back.
      await page.getByTestId("chat-mobile-tab-related").click();
      await expect(messageList).toBeHidden();
      await page.getByTestId("chat-mobile-tab-chat").click();
      await expect(messageList).toBeVisible();

      // The list should not have snapped back to the bottom — i.e. the auto
      // scroll-to-bottom effect must not have re-fired on re-show (the pane
      // is hidden via display: none, never unmounted).
      const stateAfter = await messageList.evaluate((el) => ({
        scrollTop: el.scrollTop,
        scrollHeight: el.scrollHeight,
        clientHeight: el.clientHeight,
      }));
      expect(stateAfter.scrollTop).toBeLessThan(stateAfter.scrollHeight - stateAfter.clientHeight);
      expect(Math.abs(stateAfter.scrollTop - targetScroll)).toBeLessThan(4);

      const documentScrollTop = await page.evaluate(
        () => document.scrollingElement?.scrollTop ?? window.scrollY,
      );
      expect(documentScrollTop).toBe(0);
    });
  });

  test.describe("at 1280x720 viewport", () => {
    test.use({ viewport: { width: 1280, height: 720 } });

    test("desktop hides the mobile tab bar and shows the right-panel's own tabs @mobile:chat-tabs", async ({
      authenticatedPage: page,
    }) => {
      await setSidebarHidden(page);
      await mockChatRoutes(page);
      await page.goto(`/chat/${CONVERSATION_ID}`);

      // MobileTabBar is rendered but hidden via CSS at desktop widths.
      await expect(page.getByTestId("chat-mobile-tab-chat")).toBeHidden();
      await expect(page.getByTestId("chat-mobile-tab-related")).toBeHidden();
      await expect(page.getByTestId("chat-mobile-tab-settings")).toBeHidden();

      // The right-panel's own Related/Settings tabs are visible.
      await expect(page.getByTestId("chat-rail-tab-related")).toBeVisible();
      await expect(page.getByTestId("chat-rail-tab-settings")).toBeVisible();

      // Message list and right panel both visible side-by-side.
      await expect(page.getByTestId("chat-message-list")).toBeVisible();
      await expect(page.getByText("No issues referenced by this chat yet.")).toBeVisible();
    });
  });
});
