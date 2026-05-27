-- Phase 4a of the actor-system overhaul
-- (designs/actor-system-overhaul.md §8.3 step 5, §11 row 4). Adds a
-- typed `assignee_principal` JSON column alongside the existing
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
-- `agents` table (case-sensitive `=`, both deleted and non-deleted
-- agents — once an agent name, always an agent name for the purpose
-- of legacy attribution). Historically users and agents have been
-- conflated in `Issue.assignee` strings, so we split them out via
-- this string match rather than blindly lifting every bare string to
-- `Principal::User`.

ALTER TABLE issues_v2 ADD COLUMN assignee_principal TEXT;

UPDATE issues_v2
SET assignee_principal = CASE
        -- users/<x>
        WHEN substr(assignee, 1, 6) = 'users/'
             AND length(assignee) > 6
             AND substr(assignee, 7) NOT LIKE '%/%'
             AND substr(assignee, 7) NOT LIKE '% %'
             AND substr(assignee, 7) NOT LIKE '%' || char(9) || '%'
             AND substr(assignee, 7) NOT LIKE '%' || char(10) || '%'
             AND substr(assignee, 7) NOT LIKE '%' || char(13) || '%'
            THEN json_object('User', json_object('name', substr(assignee, 7)))
        -- agents/<x>
        WHEN substr(assignee, 1, 7) = 'agents/'
             AND length(assignee) > 7
             AND substr(assignee, 8) NOT LIKE '%/%'
             AND substr(assignee, 8) NOT LIKE '% %'
             AND substr(assignee, 8) NOT LIKE '%' || char(9) || '%'
             AND substr(assignee, 8) NOT LIKE '%' || char(10) || '%'
             AND substr(assignee, 8) NOT LIKE '%' || char(13) || '%'
            THEN json_object('Agent', json_object('name', substr(assignee, 8)))
        -- bare <name> matching a known agent
        WHEN assignee != ''
             AND assignee NOT LIKE '%/%'
             AND assignee NOT LIKE '% %'
             AND assignee NOT LIKE '%' || char(9) || '%'
             AND assignee NOT LIKE '%' || char(10) || '%'
             AND assignee NOT LIKE '%' || char(13) || '%'
             AND EXISTS (SELECT 1 FROM agents WHERE agents.name = issues_v2.assignee)
            THEN json_object('Agent', json_object('name', assignee))
        -- bare <username>
        WHEN assignee != ''
             AND assignee NOT LIKE '%/%'
             AND assignee NOT LIKE '% %'
             AND assignee NOT LIKE '%' || char(9) || '%'
             AND assignee NOT LIKE '%' || char(10) || '%'
             AND assignee NOT LIKE '%' || char(13) || '%'
            THEN json_object('User', json_object('name', assignee))
        ELSE NULL
    END
WHERE assignee IS NOT NULL AND assignee_principal IS NULL;
