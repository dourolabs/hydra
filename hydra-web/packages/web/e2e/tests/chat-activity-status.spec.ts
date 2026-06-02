import type { Page } from "@playwright/test";
import { test, expect } from "../fixtures/auth";

// `@chat:activity-status` — the transient activity indicator below the
// message thread should:
//   1. Appear as `Thinking…` once the user sends a message (driven by the
//      optimistic-merged `user_message` tail).
//   2. Transition to a tool label (e.g. `Searching code`) when the worker
//      emits a `ToolUse` SessionEvent live.
//   3. Disappear once an `AssistantMessage` lands.
//
// In real prod usage `useSSE` invalidates `["sessionEvents", sid]` whenever
// the server pushes a `session_event_created` event, driving the refetch +
// re-render. In the e2e dev setup that flow is currently broken: the
// mock-server's `/v1/events` filter compares `types=...` (entity-category
// names: `sessions`, `issues`, …) against the FULL event name
// (`session_event_created`), so the SSE message never reaches the browser
// even though `appendSessionEvent` emits it correctly. Fixing the mock-
// server filter is out of scope for this patch and tracked as a follow-up.
//
// In the meantime, after appending the synthesised event via the dev
// endpoint, we drive `queryClient.invalidateQueries` directly through the
// dev-only `window.__hydraQueryClient` export to refetch `["sessionEvents",
// sid]`. The derive → re-render path under test is unchanged; only the
// initial invalidation trigger is shimmed. Unit tests for `useSSE` cover the
// SSE-driven invalidation path on its own.

const SEED_CONVERSATION_ID = "c-seed00001"; // dev-user's only active seed convo
const SEED_SESSION_ID = "t-seed00016";

async function appendSessionEventToMock(
  request: import("@playwright/test").APIRequestContext,
  sessionId: string,
  body: Record<string, unknown>,
): Promise<void> {
  const res = await request.post(
    `http://localhost:8080/v1/dev/sessions/${sessionId}/events`,
    {
      headers: { Authorization: "Bearer dev-token-12345" },
      data: body,
    },
  );
  if (!res.ok()) {
    throw new Error(
      `appendSessionEventToMock failed: ${res.status()} ${await res.text()}`,
    );
  }
}

async function invalidateSessionEvents(page: Page, sessionId: string): Promise<void> {
  await page.evaluate((sid) => {
    const qc = (
      window as unknown as {
        __hydraQueryClient?: {
          invalidateQueries: (opts: { queryKey: unknown[] }) => Promise<void>;
        };
      }
    ).__hydraQueryClient;
    qc?.invalidateQueries({ queryKey: ["sessionEvents", sid] });
  }, sessionId);
}

test.describe("Chat activity indicator @chat:activity-status", () => {
  test(
    "appears as Thinking…, transitions to a ToolUse label, then disappears on AssistantMessage @chat:activity-status",
    async ({ authenticatedPage: page, request }) => {
      await page.goto(`/chat/${SEED_CONVERSATION_ID}`);
      await expect(page.getByTestId("chat-message-list")).toBeVisible();

      // No indicator before the user has spoken (tail is an assistant_message
      // from the seed fixture).
      await expect(page.getByTestId("chat-activity-indicator")).toHaveCount(0);

      // Send a message via the composer. The optimistic merge makes the
      // tail a user_message immediately, so the indicator pops as Thinking…
      // BEFORE the SSE roundtrip — proves the optimistic path is wired up.
      const composer = page.getByPlaceholder("Type a message…");
      await composer.fill("Find references to the OAuth handler");
      await page.getByRole("button", { name: "Send" }).click();

      const indicatorText = page.getByTestId("chat-activity-indicator-text");
      await expect(indicatorText).toHaveText("Thinking…");

      // Worker emits a Grep tool_use → indicator should switch to
      // `Searching code` without remounting (same testid still resolves).
      await appendSessionEventToMock(request, SEED_SESSION_ID, {
        type: "tool_use",
        tool_name: "Grep",
        payload: { pattern: "OAuth", path: "src/" },
        timestamp: new Date(Date.now() + 1000).toISOString(),
      });
      await invalidateSessionEvents(page, SEED_SESSION_ID);
      await expect(indicatorText).toHaveText("Searching code");

      // Final assistant message lands → indicator hides.
      await appendSessionEventToMock(request, SEED_SESSION_ID, {
        type: "assistant_message",
        content: "Found 3 references — let me know which to focus on.",
        timestamp: new Date(Date.now() + 2000).toISOString(),
      });
      await invalidateSessionEvents(page, SEED_SESSION_ID);
      await expect(page.getByTestId("chat-activity-indicator")).toHaveCount(0);

      // The indicator never appeared as a transcript row (it lives below the
      // thread, not inside it). Final assistant message is part of history.
      const list = page.getByTestId("chat-message-list");
      await expect(list).toContainText(
        "Found 3 references — let me know which to focus on.",
      );
      expect(
        await list.locator('[data-testid="chat-activity-indicator"]').count(),
      ).toBe(0);
    },
  );
});
