CREATE TABLE IF NOT EXISTS hydra.agents (
    name TEXT PRIMARY KEY,
    prompt_path TEXT NOT NULL,
    max_tries INT NOT NULL DEFAULT 3,
    max_simultaneous INT NOT NULL DEFAULT 2147483647,
    is_assignment_agent BOOLEAN NOT NULL DEFAULT FALSE,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Enforce at most one non-deleted assignment agent.
CREATE UNIQUE INDEX agents_assignment_idx
    ON hydra.agents (is_assignment_agent)
    WHERE is_assignment_agent = TRUE AND NOT deleted;
