-- Refactor messages_v2: add recipient column, make sender nullable,
-- backfill recipient from conversation_id + sender, then drop conversation_id.

-- Step 1: Add the recipient column (nullable initially for backfill)
ALTER TABLE metis.messages_v2 ADD COLUMN IF NOT EXISTS recipient TEXT;

-- Step 2: Make sender nullable
ALTER TABLE metis.messages_v2 ALTER COLUMN sender DROP NOT NULL;

-- Step 3: Backfill recipient from conversation_id and sender.
-- The conversation_id format is "actor1+actor2" where actors are sorted lexicographically.
-- The recipient is the other actor (the one that is not the sender).
UPDATE metis.messages_v2
SET recipient = CASE
    WHEN split_part(conversation_id, '+', 1) = sender THEN split_part(conversation_id, '+', 2)
    ELSE split_part(conversation_id, '+', 1)
END
WHERE recipient IS NULL AND conversation_id IS NOT NULL;

-- Step 4: Make recipient NOT NULL now that it's backfilled
ALTER TABLE metis.messages_v2 ALTER COLUMN recipient SET NOT NULL;

-- Step 5: Add index on recipient
CREATE INDEX IF NOT EXISTS idx_messages_v2_recipient ON metis.messages_v2 (recipient);

-- Step 6: Add index on sender (already exists, but ensure it covers nullable values)
-- The existing idx_messages_v2_sender index should still work.

-- Step 7: Drop the conversation_id column and its index
DROP INDEX IF EXISTS metis.idx_messages_v2_conversation;
ALTER TABLE metis.messages_v2 DROP COLUMN IF EXISTS conversation_id;
