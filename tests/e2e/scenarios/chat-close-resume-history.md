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
11. Click the "End Chat" button in the chat header. The UI auto-navigates back to the chat list page (`/chat`) once the close completes — there is no in-conversation status feedback to look for.
12. Click on the now-Closed conversation in the chat list to reopen it (this brings you back into the message list view so the next checks and send can be performed).
13. Verify the message list still shows all 3 prior user messages and all 3 prior assistant replies (no messages were lost when the conversation was closed). Do this BEFORE sending msg4 so the close/resume boundary is observable between msg3 and msg4.
14. Send **msg4**: `What's my name and what do I work on?` (sending a message to the now-Closed conversation auto-resumes it; no explicit resume action is required).
15. Verify a "Resumed" inline system event appears in the message list between msg3's assistant reply and msg4. Inspect the event payload (e.g. via the dashboard event inspector or the underlying API) and confirm `source == "native"` — the prior worker's graceful close uploaded `session_state` before exiting, so the new worker takes the native-restore path rather than the transcript-replay fallback.
16. Wait for the assistant reply to appear (allow up to 2 minutes for the resumed worker to spin up).
17. Inspect the 4th assistant reply.

## Expected Results

- After step 16, the chat message list contains exactly 4 user messages and 4 assistant messages (plus event rows for the Closed/Resumed system events between msg3's reply and msg4).
- No user or assistant message appears more than once — the resumed worker must not have re-replied to msg1/msg2/msg3.
- The 4th assistant reply must reference both **Alice** (proving the worker has msg1 in its context) and **Rust** (proving the worker has msg3 in its context).
- The chat input remains enabled even when the conversation status is Closed — sending a message to a Closed conversation is what auto-resumes it (there is no explicit resume control in the UI).
- The conversation history (user messages, assistant replies, Closed event, post-msg4 Resumed inline system event) is preserved and displayed in order across the close/resume boundary.
- The Resumed inline event reports `source = native` (proving the close→reopen path took the native-restore branch backed by the graceful End Chat upload, not the transcript-replay fallback).
- No JavaScript errors or broken layouts throughout.
