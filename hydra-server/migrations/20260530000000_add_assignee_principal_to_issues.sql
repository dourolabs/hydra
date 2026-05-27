-- Phase 4a of the actor-system overhaul
-- (designs/actor-system-overhaul.md §8.3 step 5, §11 row 4). Adds a
-- typed `assignee_principal` JSONB column alongside the existing
-- `assignee` string. This PR dual-writes both columns from the app
-- layer; Phase 4b switches the wire format to read from the typed
-- column; Phase 7 (release-gated) drops the legacy `assignee` column.
--
-- Inline-SQL backfill (approach (a) in the issue): the heuristic
-- mirrors `domain::issues::parse_assignee_as_principal` for the cases
-- the SQL dialect can express cleanly:
--   * `users/<x>`  with valid <x> -> {"User": {"name":"<x>"}}
--   * `agents/<x>` with valid <x> -> {"Agent":{"name":"<x>"}}
--   * bare `<x>` that matches an `agents.name` row
--                                -> {"Agent":{"name":"<x>"}}
--   * other bare `<x>` with valid <x>
--                                -> {"User": {"name":"<x>"}}
-- The `external/<sys>/<x>` case is left NULL by the migration — no
-- real existing rows are expected to use that form yet, and the
-- next-write dual-write path will populate it when an `external/...`
-- assignee is rewritten. Anything else (whitespace, slashes in the
-- wrong place, empty username segment) stays NULL.
--
-- The bare-name → agent classification is driven by the live
-- `metis.agents` table (case-sensitive `=`, both deleted and
-- non-deleted agents — once an agent name, always an agent name for
-- the purpose of legacy attribution). Historically users and agents
-- have been conflated in `Issue.assignee` strings, so we split them
-- out via this string match rather than blindly lifting every bare
-- string to `Principal::User`.
--
-- The `[ \t\n\r]` regex character class is checked via `~`; a NULL
-- assignee is filtered out in the WHERE clause so the LIKE/regex
-- predicates only see non-null input.

ALTER TABLE metis.issues_v2 ADD COLUMN IF NOT EXISTS assignee_principal JSONB;

UPDATE metis.issues_v2
SET assignee_principal = CASE
        -- users/<x>
        WHEN substring(assignee FROM 1 FOR 6) = 'users/'
             AND length(assignee) > 6
             AND substring(assignee FROM 7) !~ '[/[:space:]]'
            THEN jsonb_build_object('User', jsonb_build_object('name', substring(assignee FROM 7)))
        -- agents/<x>
        WHEN substring(assignee FROM 1 FOR 7) = 'agents/'
             AND length(assignee) > 7
             AND substring(assignee FROM 8) !~ '[/[:space:]]'
            THEN jsonb_build_object('Agent', jsonb_build_object('name', substring(assignee FROM 8)))
        -- bare <name> matching a known agent
        WHEN assignee <> ''
             AND assignee !~ '[/[:space:]]'
             AND EXISTS (SELECT 1 FROM metis.agents WHERE name = issues_v2.assignee)
            THEN jsonb_build_object('Agent', jsonb_build_object('name', assignee))
        -- bare <username>
        WHEN assignee <> ''
             AND assignee !~ '[/[:space:]]'
            THEN jsonb_build_object('User', jsonb_build_object('name', assignee))
        ELSE NULL
    END
WHERE assignee IS NOT NULL AND assignee_principal IS NULL;
