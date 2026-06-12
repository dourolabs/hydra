import { test, expect } from "../fixtures/auth";

// `@chat:system-event` — the new `SessionEvent` variant
// `system_event { kind: { kind: "child_unblocked", child_id, new_status } }`
// is rendered as a `SystemEventBubble` in the conversation timeline:
//
//   - structured chip with the child issue's title (resolved via the existing
//     `useIssue` query) and the child's resolved StatusChip label
//   - clickable link to `/issues/<child_id>`
//   - unknown kinds fall back to a generic "System event" line (not exercised
//     here — covered by the SystemEventBubble unit test)
//
// The seed fixture wires conversation `c-sysev01` → session `t-sysev01`,
// whose event log contains a `child_unblocked` event for child issue
// `i-sysevch` (title: "Re-index search corpus", status: closed). See
// `packages/mock-server/fixtures/seed.json` for the exact shape — the
// JSON keys here must match the ts-rs serialization of
// `SessionEvent::SystemEvent` and `SystemEventKind::ChildUnblocked`.

const SEED_CONVERSATION_ID = "c-sysev01";
const SEED_CHILD_ISSUE_ID = "i-sysevch";

test.describe("Chat SystemEventBubble @chat:system-event", () => {
  test(
    "renders the seeded ChildUnblocked event as a structured chip linking to the child issue @chat:system-event",
    async ({ authenticatedPage: page }) => {
      await page.goto(`/chat/${SEED_CONVERSATION_ID}`);
      await expect(page.getByTestId("chat-message-list")).toBeVisible();

      const bubble = page.getByTestId("system-event-bubble");
      await expect(bubble).toHaveCount(1);
      await expect(bubble).toHaveAttribute("data-kind", "child_unblocked");

      const chip = page.getByTestId("system-event-child-unblocked-chip");
      await expect(chip).toHaveAttribute("data-child-id", SEED_CHILD_ISSUE_ID);
      await expect(chip).toHaveAttribute("href", `/issues/${SEED_CHILD_ISSUE_ID}`);
      // Child issue title is resolved via the existing useIssue query, not
      // baked into the event payload — verifies the chip actually consumes
      // the issue cache rather than blindly echoing the JSON.
      await expect(chip).toContainText("Re-index search corpus");
      // The new status label is the StatusChip's resolved label, not the
      // raw status key from the event payload (the event carries the key
      // "closed"; the chip should show the project's "Closed" label).
      await expect(chip).toContainText("Closed");

      // SystemEventBubble is NOT a user-message bubble — the bubble must
      // live in the transcript without being styled as the user's input.
      // The seeded conversation has 2 assistant messages, 1 user message,
      // and 1 system event; the test guards the system event renders as
      // its own component, not as a misclassified user message.
      const list = page.getByTestId("chat-message-list");
      await expect(list).toContainText("Kick off the search re-index task");
      await expect(list).toContainText("Child task closed");

      // Clicking the chip navigates to the child issue's detail page.
      await chip.click();
      await expect(page).toHaveURL(new RegExp(`/issues/${SEED_CHILD_ISSUE_ID}$`));
    },
  );
});
