-- Sister to the Postgres `20260722000000_split_agents_max_simultaneous.sql`.
-- Split `agents.max_simultaneous` into per-mode caps
-- `max_simultaneous_interactive` / `max_simultaneous_headless` so a SWE
-- agent can be limited to N concurrent live-preview conversations without
-- throttling its headless work. SQLite >=3.25 supports `RENAME COLUMN`,
-- so the rename + add-column + back-fill sequence runs natively.

ALTER TABLE agents
    RENAME COLUMN max_simultaneous TO max_simultaneous_headless;

ALTER TABLE agents
    ADD COLUMN max_simultaneous_interactive INTEGER NOT NULL DEFAULT 2147483647;

UPDATE agents
    SET max_simultaneous_interactive = max_simultaneous_headless;
