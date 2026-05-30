-- Drop the now-dead `metis.conversation_events_v2` table. The lifecycle
-- (Suspending/Resumed/Closed) is fully expressed as `Conversation.status`
-- transitions on `metis.conversations_v2`; chat content lives on
-- `metis.session_events_v2`. See issue for the full migration rationale.
DROP TABLE IF EXISTS metis.conversation_events_v2;
