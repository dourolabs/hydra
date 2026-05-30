-- Drop the now-dead `conversation_events` table. The lifecycle
-- (Suspending/Resumed/Closed) is fully expressed as `Conversation.status`
-- transitions on the `conversations` table; chat content lives on
-- `session_events`. See issue for the full migration rationale.
DROP TABLE IF EXISTS conversation_events;
