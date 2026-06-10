-- baseline-version: 20260618000000
-- SQLite pre-drop-is-assignment-agent baseline. INSERTs are valid against
-- the schema state at sqlite migration
-- `20260618000000_add_clear_assignee_to_default_terminal_statuses.sql`,
-- immediately before `20260619000000_drop_is_assignment_agent.sql`.
--
-- Captures a representative slice of the `agents` table at the moment
-- before the assignment-agent column is dropped so the rebuild-and-rename
-- migration is exercised against real rows with the column populated.
-- Sister to the postgres baseline at the same version. The 20260618000000
-- clear_assignee migration mutates `statuses.on_enter` and does not touch
-- `agents`, so this baseline's seed rows are unaffected by the rebase.

INSERT INTO agents (
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
    -- The historical assignment agent: `is_assignment_agent = 1` exercises
    -- the column-drop path on a flagged row.
    ('pm-baseline', '/agents/pm-baseline/prompt.md', 3, 2147483647, 1, 0,
     '2026-06-01T00:00:00.000+00:00', '2026-06-01T00:00:00.000+00:00',
     '[]', NULL, 0),
    -- A non-flagged agent with all post-init columns populated; covers
    -- the common case and verifies the rebuild preserves
    -- `is_default_conversation_agent`, `secrets`, and `mcp_config_path`.
    ('chat-baseline', '/agents/chat-baseline/prompt.md', 5, 10, 0, 0,
     '2026-06-02T00:00:00.000+00:00', '2026-06-02T00:00:00.000+00:00',
     '["OPENAI_API_KEY"]', '/agents/chat-baseline/mcp.json', 1),
    -- A soft-deleted row: the rebuild must preserve `deleted = 1` so
    -- `list_agents` keeps filtering it out.
    ('deleted-baseline', '/agents/deleted-baseline/prompt.md', 3, 1, 0, 1,
     '2026-06-03T00:00:00.000+00:00', '2026-06-03T00:00:00.000+00:00',
     '[]', NULL, 0);
