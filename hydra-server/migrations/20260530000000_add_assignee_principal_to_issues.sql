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
--   * `users/<x>`  with valid <x> -> {"kind":"user","name":"<x>"}
--   * `agents/<x>` with valid <x> -> {"kind":"agent","name":"<x>"}
--   * bare `<x>` with valid <x>   -> {"kind":"user","name":"<x>"}
-- The `external/<sys>/<x>` case is left NULL by the migration — no
-- real existing rows are expected to use that form yet, and the
-- next-write dual-write path will populate it when an `external/...`
-- assignee is rewritten. Anything else (whitespace, slashes in the
-- wrong place, empty username segment) stays NULL.
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
            THEN jsonb_build_object('kind', 'user', 'name', substring(assignee FROM 7))
        -- agents/<x>
        WHEN substring(assignee FROM 1 FOR 7) = 'agents/'
             AND length(assignee) > 7
             AND substring(assignee FROM 8) !~ '[/[:space:]]'
            THEN jsonb_build_object('kind', 'agent', 'name', substring(assignee FROM 8))
        -- bare <username>
        WHEN assignee <> ''
             AND assignee !~ '[/[:space:]]'
            THEN jsonb_build_object('kind', 'user', 'name', assignee)
        ELSE NULL
    END
WHERE assignee IS NOT NULL AND assignee_principal IS NULL;
