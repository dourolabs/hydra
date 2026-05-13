ALTER TABLE metis.agents ADD COLUMN is_default_conversation_agent BOOLEAN NOT NULL DEFAULT FALSE;

-- Enforce at most one non-deleted default conversation agent.
CREATE UNIQUE INDEX agents_default_conversation_idx
    ON metis.agents (is_default_conversation_agent)
    WHERE is_default_conversation_agent = TRUE AND NOT deleted;
