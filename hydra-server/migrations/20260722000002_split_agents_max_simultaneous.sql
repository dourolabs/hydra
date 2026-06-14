-- Split `agents.max_simultaneous` into
-- `max_simultaneous_interactive` / `max_simultaneous_headless` so an agent's
-- live preview (conversation-mode) sessions can be capped independently from
-- its headless work. Existing values back-fill into BOTH columns so the
-- pre-migration behaviour (one combined cap) is preserved per agent until an
-- operator explicitly lowers one of the new caps.

ALTER TABLE metis.agents
    RENAME COLUMN max_simultaneous TO max_simultaneous_headless;

ALTER TABLE metis.agents
    ADD COLUMN max_simultaneous_interactive INT NOT NULL DEFAULT 2147483647;

UPDATE metis.agents
    SET max_simultaneous_interactive = max_simultaneous_headless;
