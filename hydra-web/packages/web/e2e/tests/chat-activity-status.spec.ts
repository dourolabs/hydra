import type { Page } from "@playwright/test";
import { test, expect } from "../fixtures/auth";

// `@chat:activity-status` — the inline `ChatActivityLine` rendered as the
// trailing transcript item inside `ChatMessageList` should:
//   1. Appear as `Thinking…` once the user sends a message (driven by the
//      optimistic-merged `user_message` tail).
//   2. Transition to a tool label (e.g. `Searching code`) when the worker
//      emits a `ToolUse` SessionEvent live.
//   3. Surface a tool's `payload.description` in the detail span when set
//      (verb remains the friendly tool label).
//   4. Settle into a `done`-state summary once an `AssistantMessage` lands —
//      the line stays inside the transcript with the historical step count,
//      since the run terminated with completed steps the user can review.
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

test.describe("Chat activity line @chat:activity-status", () => {
  test(
    "appears as Thinking…, transitions through ToolUse labels, then settles to a done summary on AssistantMessage @chat:activity-status",
    async ({ authenticatedPage: page, request }) => {
      await page.goto(`/chat/${SEED_CONVERSATION_ID}`);
      await expect(page.getByTestId("chat-message-list")).toBeVisible();

      // No activity line before the user has spoken (tail is an
      // assistant_message from the seed fixture, no tool steps to summarise).
      await expect(page.getByTestId("chat-activity-line")).toHaveCount(0);

      // Send a message via the composer. The optimistic merge makes the
      // tail a user_message immediately, so the line pops as Thinking…
      // BEFORE the SSE roundtrip — proves the optimistic path is wired up.
      const composer = page.getByPlaceholder("Type a message…");
      await composer.fill("Find references to the OAuth handler");
      await page.getByRole("button", { name: "Send" }).click();

      const activityLine = page.getByTestId("chat-activity-line");
      const verb = page.getByTestId("chat-activity-line-verb");
      const detail = page.getByTestId("chat-activity-line-detail");

      await expect(verb).toHaveText("Thinking…");
      await expect(activityLine).toHaveAttribute("data-state", "live");

      // Worker emits a Grep tool_use (no `description` in payload) →
      // verb should switch to the TOOL_LABELS entry `Searching code` and no
      // detail span renders (Grep payload has no description).
      await appendSessionEventToMock(request, SEED_SESSION_ID, {
        type: "tool_use",
        tool_name: "Grep",
        payload: { pattern: "OAuth", path: "src/" },
        timestamp: new Date(Date.now() + 1000).toISOString(),
      });
      await invalidateSessionEvents(page, SEED_SESSION_ID);
      await expect(verb).toHaveText("Searching code");
      await expect(detail).toHaveCount(0);

      // Worker emits a Bash tool_use whose payload carries a human-readable
      // `description`. The new component splits the friendly tool label and
      // the description across `verb` and `detail` spans (per i-jxamlakh:
      // description surfaces as the detail; verb stays the TOOL_LABELS entry).
      await appendSessionEventToMock(request, SEED_SESSION_ID, {
        type: "tool_use",
        tool_name: "Bash",
        payload: {
          command: "rg -n OAuth packages/web/src",
          description: "Locate OAuth handler usages",
        },
        timestamp: new Date(Date.now() + 1500).toISOString(),
      });
      await invalidateSessionEvents(page, SEED_SESSION_ID);
      await expect(verb).toHaveText("Running command");
      await expect(detail).toHaveText("Locate OAuth handler usages");

      // Final assistant message lands → run terminates. With historical
      // steps present the line stays visible as a `done`-state summary
      // ("2 steps" + total duration) so the user can review what happened.
      await appendSessionEventToMock(request, SEED_SESSION_ID, {
        type: "assistant_message",
        content: "Found 3 references — let me know which to focus on.",
        timestamp: new Date(Date.now() + 2000).toISOString(),
      });
      await invalidateSessionEvents(page, SEED_SESSION_ID);
      await expect(activityLine).toHaveAttribute("data-state", "done");
      await expect(verb).toHaveText("2 steps");

      // The activity line lives INSIDE the message list (it's the trailing
      // transcript item, not a sibling below the thread). The final
      // assistant message is also part of the transcript.
      const list = page.getByTestId("chat-message-list");
      await expect(list).toContainText(
        "Found 3 references — let me know which to focus on.",
      );
      expect(
        await list.locator('[data-testid="chat-activity-line"]').count(),
      ).toBe(1);
    },
  );
});
