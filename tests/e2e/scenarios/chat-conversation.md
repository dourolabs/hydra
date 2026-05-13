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
9. Click the "End Chat" button in the chat header
10. Navigate back to the chat list page (`/chat`)
11. Verify the closed conversation appears in the list
12. Click on the closed conversation to reopen it
13. Verify the conversation history (user message + assistant response + closed event) is displayed
14. Type and send a new message in the chat input (sending a message to a Closed conversation auto-resumes it; no explicit resume action is required)
15. Verify the new user message appears in the chat message list
16. Verify a "Resumed" inline system event appears in the message list (between the prior "Closed" event and the new user message)
17. Wait for an assistant response to the new message
18. Click the "End Chat" button again to clean up

## Expected Results

- Chat list page loads without errors
- New conversations can be created with an initial message
- User messages appear immediately in the UI
- Assistant responses appear after agent processing (may take up to 2 minutes)
- Conversations can be ended via the "End Chat" button; the ended conversation still appears in the chat list and its history can be reopened
- Sending a message to a Closed conversation auto-resumes it; a "Resumed" inline system event appears in the message list and the assistant replies to the new message
- Conversation history persists across close/resume cycles
- No JavaScript errors or broken layouts throughout
