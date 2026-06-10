-- baseline-version: 20260618000000
-- Postgres pre-drop-is-assignment-agent baseline. INSERTs are valid
-- against the schema state at sqlx migration
-- `20260618000000_add_clear_assignee_to_default_terminal_statuses.sql`,
-- immediately before `20260619000000_drop_is_assignment_agent.sql`.
--
-- Captures a representative slice of the `metis.agents` table at the
-- moment before the assignment-agent column is dropped so the
-- `ALTER TABLE ... DROP COLUMN` is exercised against real rows with
-- the column populated. Sister to the sqlite baseline at the same
-- version. The 20260618000000 clear_assignee migration mutates the
-- `metis.statuses.on_enter` JSONB and does not touch `metis.agents`,
-- so this baseline's seed rows are unaffected by the rebase.

INSERT INTO metis.agents (
    name,
    prompt_path,
    max_tries,
    max_simultaneous,
    is_assignment_agent,
    deleted,
    created_at,
    updated_at,
    secrets,
    mcp_config_path,
    is_default_conversation_agent
)
VALUES
    -- The historical assignment agent.
    ('pm-baseline', '/agents/pm-baseline/prompt.md', 3, 2147483647, TRUE, FALSE,
     '2026-06-01T00:00:00Z'::timestamptz, '2026-06-01T00:00:00Z'::timestamptz,
     '[]'::jsonb, NULL, FALSE),
    -- A non-flagged agent with all post-init columns populated.
    ('chat-baseline', '/agents/chat-baseline/prompt.md', 5, 10, FALSE, FALSE,
     '2026-06-02T00:00:00Z'::timestamptz, '2026-06-02T00:00:00Z'::timestamptz,
     '["OPENAI_API_KEY"]'::jsonb, '/agents/chat-baseline/mcp.json', TRUE),
    -- A soft-deleted row: the DROP COLUMN must preserve `deleted = TRUE`.
    ('deleted-baseline', '/agents/deleted-baseline/prompt.md', 3, 1, FALSE, TRUE,
     '2026-06-03T00:00:00Z'::timestamptz, '2026-06-03T00:00:00Z'::timestamptz,
     '[]'::jsonb, NULL, FALSE);
