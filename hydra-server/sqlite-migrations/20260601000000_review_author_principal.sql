-- Phase 5b of the actor-system overhaul
-- (designs/actor-system-overhaul.md §8.2, §11 row 5). Rewrites
-- `patches_v2.reviews` JSON blobs so every embedded review's
-- `author` field becomes a typed `Principal` object instead of a
-- bare string.
--
-- The heuristic mirrors `domain::patches::legacy_author_to_principal`
-- / `Principal::parse_legacy_assignee` for the syntactic forms the
-- SQLite JSON1 dialect can express cleanly:
--   * "users/<x>"    with valid <x> -> {"User": {"name":"<x>"}}
--   * "agents/<x>"   with valid <x> -> {"Agent":{"name":"<x>"}}
--   * bare "<x>" matching `agents.name`
--                                   -> {"Agent":{"name":"<x>"}}
--   * other bare "<x>" with valid <x>
--                                   -> {"User": {"name":"<x>"}}
-- The `"external/<sys>/<x>"` case is left unchanged by the
-- migration -- no real existing rows are expected to use that form
-- yet, and the runtime poller (re-)writes typed `External`
-- principals on next sync. Anything else (empty string, embedded
-- whitespace) stays as-is; the Rust-side custom deserializer logs
-- a warning and the patch falls through `parse_legacy_assignee`.
--
-- The bare-name → agent classification is driven by the live
-- `agents` table (case-sensitive `=`, both deleted and non-deleted
-- agents). Historically users and agents have been conflated in
-- `Review.author` strings, so we split them out via this string
-- match rather than blindly lifting every bare string to
-- `Principal::User`.
--
-- Per [[migrations]]: the SELECT explicitly names every column on
-- the `patches_v2` table. New columns (post-Phase-5b) require
-- updating this UPDATE statement.
--
-- The reviews column is JSON-encoded TEXT (per the
-- 20260307000000_init.sql schema). We walk every JSON array
-- element with json_each() and rewrite the `author` key.

UPDATE patches_v2
SET reviews = (
    SELECT json_group_array(
        json_object(
            'contents',     json_extract(value, '$.contents'),
            'is_approved',  json(coalesce(
                json_extract(value, '$.is_approved'),
                'false'
            )),
            'author', CASE
                -- Already typed (Phase 5b shape: object): leave untouched.
                WHEN json_type(value, '$.author') = 'object'
                    THEN json(json_extract(value, '$.author'))
                -- `users/<x>` with a syntactically-valid <x>.
                WHEN json_type(value, '$.author') = 'text'
                     AND substr(json_extract(value, '$.author'), 1, 6) = 'users/'
                     AND length(json_extract(value, '$.author')) > 6
                     AND substr(json_extract(value, '$.author'), 7) NOT LIKE '%/%'
                     AND substr(json_extract(value, '$.author'), 7) NOT LIKE '% %'
                     AND substr(json_extract(value, '$.author'), 7) NOT LIKE '%' || char(9)  || '%'
                     AND substr(json_extract(value, '$.author'), 7) NOT LIKE '%' || char(10) || '%'
                     AND substr(json_extract(value, '$.author'), 7) NOT LIKE '%' || char(13) || '%'
                    THEN json_object(
                        'User',
                        json_object('name', substr(json_extract(value, '$.author'), 7))
                    )
                -- `agents/<x>` with a syntactically-valid <x>.
                WHEN json_type(value, '$.author') = 'text'
                     AND substr(json_extract(value, '$.author'), 1, 7) = 'agents/'
                     AND length(json_extract(value, '$.author')) > 7
                     AND substr(json_extract(value, '$.author'), 8) NOT LIKE '%/%'
                     AND substr(json_extract(value, '$.author'), 8) NOT LIKE '% %'
                     AND substr(json_extract(value, '$.author'), 8) NOT LIKE '%' || char(9)  || '%'
                     AND substr(json_extract(value, '$.author'), 8) NOT LIKE '%' || char(10) || '%'
                     AND substr(json_extract(value, '$.author'), 8) NOT LIKE '%' || char(13) || '%'
                    THEN json_object(
                        'Agent',
                        json_object('name', substr(json_extract(value, '$.author'), 8))
                    )
                -- Bare `<x>` matching a known agent.
                WHEN json_type(value, '$.author') = 'text'
                     AND json_extract(value, '$.author') != ''
                     AND json_extract(value, '$.author') NOT LIKE '%/%'
                     AND json_extract(value, '$.author') NOT LIKE '% %'
                     AND json_extract(value, '$.author') NOT LIKE '%' || char(9)  || '%'
                     AND json_extract(value, '$.author') NOT LIKE '%' || char(10) || '%'
                     AND json_extract(value, '$.author') NOT LIKE '%' || char(13) || '%'
                     AND EXISTS (
                         SELECT 1 FROM agents
                         WHERE agents.name = json_extract(value, '$.author')
                     )
                    THEN json_object(
                        'Agent',
                        json_object('name', json_extract(value, '$.author'))
                    )
                -- Bare `<username>` (most pre-Phase-5b reviews).
                WHEN json_type(value, '$.author') = 'text'
                     AND json_extract(value, '$.author') != ''
                     AND json_extract(value, '$.author') NOT LIKE '%/%'
                     AND json_extract(value, '$.author') NOT LIKE '% %'
                     AND json_extract(value, '$.author') NOT LIKE '%' || char(9)  || '%'
                     AND json_extract(value, '$.author') NOT LIKE '%' || char(10) || '%'
                     AND json_extract(value, '$.author') NOT LIKE '%' || char(13) || '%'
                    THEN json_object(
                        'User',
                        json_object('name', json_extract(value, '$.author'))
                    )
                -- Anything else (empty, embedded whitespace, exotic):
                -- keep the original raw value. The Rust-side custom
                -- deserializer will fall through `parse_legacy_assignee`
                -- and surface a warning per design §8.2.
                ELSE json_extract(value, '$.author')
            END,
            'submitted_at', json_extract(value, '$.submitted_at')
        )
    )
    FROM json_each(reviews)
)
WHERE reviews IS NOT NULL
  AND reviews != '[]'
  AND json_array_length(reviews) > 0;
