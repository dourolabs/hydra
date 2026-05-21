import { test, expect } from "../../fixtures/auth";

const CONVERSATION_ID = "c-mobile-header-meta";

// `created_at` is fixed in the past so `AgoTime` renders a stable, non-empty
// suffix ("5m", "1h", etc. — anything other than "now") regardless of when
// the test runs. The exact value doesn't matter, just that it parses to a
// distance ≥ 1 minute from `Date.now()` so we get a real value + "ago" pair.
const ONE_HOUR_AGO = new Date(Date.now() - 60 * 60 * 1000).toISOString();

const conversation = {
  conversation_id: CONVERSATION_ID,
  title: "Mobile header meta conversation",
  agent_name: "TestAgent",
  status: "active",
  creator: "alice",
  created_at: ONE_HOUR_AGO,
  updated_at: ONE_HOUR_AGO,
};

const conversationSummary = {
  conversation_id: CONVERSATION_ID,
  title: conversation.title,
  agent_name: conversation.agent_name,
  status: conversation.status,
  event_count: 0,
  last_event_preview: "",
  creator: conversation.creator,
  created_at: conversation.created_at,
  updated_at: conversation.updated_at,
};

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
      body: JSON.stringify([]),
    });
  });
}

test.describe("Mobile chat header meta row @mobile:chat-header-meta", () => {
  test.use({ viewport: { width: 390, height: 844 } });

  test('subheading renders "started <Xm ago>" with a visible gap between the label and the timestamp @mobile:chat-header-meta', async ({
    authenticatedPage: page,
  }) => {
    await mockChatRoutes(page);
    await page.goto(`/chat/${CONVERSATION_ID}`);

    const meta = page.getByTestId("chat-header-meta");
    await expect(meta).toBeVisible();
    const startedWrap = page.getByTestId("chat-header-started");
    await expect(startedWrap).toBeVisible();

    // ── Geometry assertion ────────────────────────────────────────────────
    // Catch the "started5m" regression by comparing the visual rect of the
    // bare "started" text node against the rect of the AgoTime element that
    // immediately follows it. If the two share an x-coordinate (or overlap)
    // the squish has come back.
    const { startedRight, agoLeft, sameRow } = await startedWrap.evaluate((wrap) => {
      const textNode = wrap.childNodes[0];
      if (!textNode || textNode.nodeType !== Node.TEXT_NODE) {
        throw new Error("expected the started wrapper's first child to be a text node");
      }
      const label = (textNode.nodeValue ?? "").trimEnd();
      if (!label.toLowerCase().startsWith("started")) {
        throw new Error(`expected text node to start with 'started', got '${label}'`);
      }
      const range = document.createRange();
      range.setStart(textNode, 0);
      range.setEnd(textNode, label.length);
      const labelRect = range.getBoundingClientRect();

      const agoEl = wrap.querySelector(":scope > span");
      if (!(agoEl instanceof HTMLElement)) {
        throw new Error("expected an AgoTime <span> child inside the started wrapper");
      }
      const agoRect = agoEl.getBoundingClientRect();
      return {
        startedRight: labelRect.right,
        agoLeft: agoRect.left,
        // Same row → comparing horizontal edges is meaningful.
        sameRow: Math.abs(labelRect.top - agoRect.top) < labelRect.height,
      };
    });

    expect(sameRow).toBe(true);
    // Strictly greater than: a zero gap is the bug we are protecting against.
    expect(agoLeft).toBeGreaterThan(startedRight);

    // ── Mobile wrap assertion ─────────────────────────────────────────────
    // At ≤768px the meta row stacks vertically; the `·` separator spans must
    // therefore not occupy any layout box. This is what guarantees that no
    // separator ever lands at the start or end of a wrapped line.
    const separatorBoxes = await meta.evaluate((node) => {
      const seps = Array.from(node.querySelectorAll("span")).filter((el) => el.textContent === "·");
      return seps.map((el) => {
        const rect = el.getBoundingClientRect();
        const style = window.getComputedStyle(el);
        return { width: rect.width, height: rect.height, display: style.display };
      });
    });
    expect(separatorBoxes.length).toBeGreaterThan(0);
    for (const box of separatorBoxes) {
      expect(box.display).toBe("none");
      expect(box.width).toBe(0);
      expect(box.height).toBe(0);
    }
  });
});
