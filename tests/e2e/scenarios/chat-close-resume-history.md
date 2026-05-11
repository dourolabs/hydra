# Scenario: Chat Close/Resume History Replay

**ID:** chat-close-resume-history
**Category:** core
**Priority:** P1
**Prerequisites:** Server running (server-init scenario passed)
**Estimated duration:** 8 minutes

## Description

Verify that closing a chat conversation and then resuming it preserves the full prior conversation history, and that after resume the assistant only replies to the next new user message rather than re-replying to earlier turns. This guards against two regressions:

1. The catch-up sent to a freshly-spawned worker after resume must include all prior user and assistant messages so the agent can reconstruct the conversation context.
2. After resume, the server must forward only the new user message to the worker — not replay msg1/msg2/msg3 — so the assistant does not generate redundant replies for the earlier turns.

## Steps (via dashboard)

1. Navigate to the Chat page via the sidebar chat icon or directly to `http://localhost:8080/chat`.
2. Click the "New Chat" button to start a new conversation.
3. Verify the new chat page loads with a message input area.
4. Send **msg1**: `My name is Alice. What's 2+2?`
5. Verify the user message appears in the chat message list (aligned right).
6. Wait for the assistant reply to appear (allow up to 2 minutes). Verify it answers the arithmetic.
7. Send **msg2**: `I'm a software engineer. What's 3+3?`
8. Wait for the assistant reply to appear.
9. Send **msg3**: `I work on Rust projects. What's 4+4?`
10. Wait for the assistant reply to appear.
11. Verify the conversation status shows as "Active" in the chat header.
12. Click the "End Chat" button in the chat header.
13. Verify the conversation status changes to "Closed".
14. Verify the chat input is disabled while the conversation is Closed.
15. Click the "Resume" button in the chat header.
16. Verify the conversation status changes back to "Active" and the chat input becomes enabled again.
17. Verify the message list still shows all 3 prior user messages and all 3 prior assistant replies (no messages were lost across close/resume).
18. Send **msg4**: `What's my name and what do I work on?`
19. Wait for the assistant reply to appear (allow up to 2 minutes for the resumed worker to spin up).
20. Inspect the 4th assistant reply.

## Expected Results

- After step 19, the chat message list contains exactly 4 user messages and 4 assistant messages (plus event rows for the Closed/Resumed system events between msg3's reply and msg4).
- No user or assistant message appears more than once — the resumed worker must not have re-replied to msg1/msg2/msg3.
- The 4th assistant reply must reference both **Alice** (proving the worker has msg1 in its context) and **Rust** (proving the worker has msg3 in its context).
- The chat input is disabled while the conversation status is Closed (after step 12) and re-enabled once the status returns to Active (after step 15).
- The conversation history (user messages, assistant replies, Closed event, Resumed event) is preserved and displayed in order across the close/resume boundary.
- No JavaScript errors or broken layouts throughout.
