import { test, expect } from "../fixtures/auth";

// Phase C step 11 cut-over: the chat read path renders from `SessionEvent`
// when any session in the conversation's resumption chain has rows, and
// falls back to the legacy `ConversationEvent` log otherwise. The mock
// server seeds c-seed00007 as a 2-session chain (t-seed00014 →
// t-seed00015) with SessionEvent fixtures; the older c-seed00001 has only
// conversation_events (legacy pre-rollout shape).

test.describe("Chat transcript source @chat:transcript-source", () => {
  test("c-seed00007 (with SessionEvent rows) renders from session_events @chat:transcript-source", async ({
    authenticatedPage: page,
  }) => {
    const sessionEventCalls: string[] = [];
    page.on("request", (request) => {
      const m = request.url().match(/\/api\/v1\/sessions\/([^/]+)\/events/);
      if (m) sessionEventCalls.push(m[1]);
    });

    await page.goto("/chat/c-seed00007");

    const list = page.getByTestId("chat-message-list");
    await expect(list).toBeVisible();
    await expect(list).toHaveAttribute("data-transcript-source", "session_events");

    // The 2-session chain renders in chronological order: first the
    // suspended session's exchange, then the resumed session's.
    await expect(list).toContainText("Start: walk me through the realtime demo deployment runbook.");
    await expect(list).toContainText("Continue: what's step 2?");
    await expect(list).toContainText("Step 2 is to roll out the CRDT persistence adapter");

    // Both sessions' event logs were fetched (parallel fan-out).
    expect(sessionEventCalls).toContain("t-seed00014");
    expect(sessionEventCalls).toContain("t-seed00015");
  });

  test("c-seed00001 (legacy / no SessionEvent rows) falls back to conversation_events @chat:transcript-source", async ({
    authenticatedPage: page,
  }) => {
    await page.goto("/chat/c-seed00001");

    const list = page.getByTestId("chat-message-list");
    await expect(list).toBeVisible();
    await expect(list).toHaveAttribute(
      "data-transcript-source",
      "conversation_events",
    );
    await expect(list).toContainText("Hi");
    await expect(list).toContainText("Here is a brief status update.");
  });
});
