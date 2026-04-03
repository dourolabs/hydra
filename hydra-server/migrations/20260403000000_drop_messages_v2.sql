-- Drop the messages_v2 table and its indexes (messages feature removed)
DROP TRIGGER IF EXISTS set_timestamp_messages_v2 ON metis.messages_v2;
DROP INDEX IF EXISTS metis.idx_messages_v2_latest;
DROP INDEX IF EXISTS metis.idx_messages_v2_sender;
DROP INDEX IF EXISTS metis.idx_messages_v2_recipient;
DROP TABLE IF EXISTS metis.messages_v2;
