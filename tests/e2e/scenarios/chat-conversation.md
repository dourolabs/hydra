# Scenario: Chat Conversation

**ID:** chat-conversation
**Category:** core
**Priority:** P1
**Prerequisites:** Server running (server-init scenario passed)
**Estimated duration:** 5 minutes

## Description

Verify the full chat conversation lifecycle through the dashboard UI. This scenario tests creating a new conversation, sending messages, receiving assistant responses, closing and resuming conversations, and verifying conversation history persistence.

## Steps (via dashboard)

1. Navigate to the Chat page via the sidebar chat icon or directly to `http://localhost:8080/chat`
2. Verify the chat list page loads (should show empty state or existing conversations)
3. Click the "New Chat" button to start a new conversation
4. Verify the new chat page loads with a message input area
5. Type a test message (e.g. "Hello, what is 2+2?") in the chat input and send it
6. Verify the user message appears in the chat message list (aligned right)
7. Wait for an assistant response to appear (allow up to 2 minutes for the agent session to start and respond)
8. Verify the assistant message appears in the chat message list (aligned left, rendered as markdown)
9. Verify the conversation status shows as "Active" in the chat header
10. Click the "End Chat" button in the chat header
11. Verify the conversation status changes to "Closed"
12. Verify the chat input is disabled after closing
13. Navigate back to the chat list page (`/chat`)
14. Verify the closed conversation appears in the list with a "Closed" status badge
15. Click on the closed conversation to reopen it
16. Verify the conversation history (user message + assistant response + closed event) is displayed
17. Click the "Resume" button in the chat header
18. Verify the conversation status changes back to "Active"
19. Send another message and verify it appears
20. End the chat again to clean up

## Expected Results

- Chat list page loads without errors
- New conversations can be created with an initial message
- User messages appear immediately in the UI
- Assistant responses appear after agent processing (may take up to 2 minutes)
- Conversations can be closed and the UI reflects the closed state
- Closed conversations can be resumed
- Conversation history persists across close/resume cycles
- No JavaScript errors or broken layouts throughout
