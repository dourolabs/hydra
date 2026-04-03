-- Drop the messages_v2 table and its indexes (messages feature removed)
DROP INDEX IF EXISTS idx_messages_v2_latest;
DROP INDEX IF EXISTS idx_messages_v2_sender;
DROP INDEX IF EXISTS idx_messages_v2_recipient;
DROP TABLE IF EXISTS messages_v2;
