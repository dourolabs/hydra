-- Phase 5b of the actor-system overhaul
-- (designs/actor-system-overhaul.md §8.2, §11 row 5). Rewrites
-- `metis.patches_v2.reviews` JSONB blobs so every embedded review's
-- `author` field becomes a typed `Principal` object instead of a
-- bare string.
--
-- The heuristic mirrors `domain::patches::legacy_author_to_principal`
-- / `Principal::parse_legacy_assignee`:
--   * "users/<x>"    with valid <x> -> {"User": {"name":"<x>"}}
--   * "agents/<x>"   with valid <x> -> {"Agent":{"name":"<x>"}}
--   * bare "<x>" matching `metis.agents.name`
--                                   -> {"Agent":{"name":"<x>"}}
--   * other bare "<x>" with valid <x>
--                                   -> {"User": {"name":"<x>"}}
-- "external/<sys>/<x>" is left unchanged; the runtime poller
-- (re-)writes typed `External` principals on next sync.
--
-- The bare-name → agent classification is driven by the live
-- `metis.agents` table (case-sensitive `=`, both deleted and
-- non-deleted agents). Historically users and agents have been
-- conflated in `Review.author` strings, so we split them out via
-- this string match rather than blindly lifting every bare string
-- to `Principal::User`.
--
-- Per [[migrations]]: this UPDATE walks every review object in
-- every `reviews` JSONB array and rewrites the `author` key. The
-- per-array rewrite is done via jsonb_set on each element. Reviews
-- whose author is already an object (i.e. already typed) are
-- preserved untouched.
--
-- We avoid `SELECT *` by listing the patches_v2 columns we read
-- explicitly.

UPDATE metis.patches_v2 AS p
SET reviews = COALESCE(
    (
        SELECT jsonb_agg(
            jsonb_build_object(
                'contents',     elem -> 'contents',
                'is_approved',  elem -> 'is_approved',
                'author', CASE
                    -- Already typed (Phase 5b shape: object).
                    WHEN jsonb_typeof(elem -> 'author') = 'object'
                        THEN elem -> 'author'
                    -- `users/<x>` with a syntactically-valid <x>.
                    WHEN jsonb_typeof(elem -> 'author') = 'string'
                         AND substring(elem ->> 'author' FROM 1 FOR 6) = 'users/'
                         AND length(elem ->> 'author') > 6
                         AND substring(elem ->> 'author' FROM 7) !~ '[/[:space:]]'
                        THEN jsonb_build_object(
                            'User',
                            jsonb_build_object('name', substring(elem ->> 'author' FROM 7))
                        )
                    -- `agents/<x>` with a syntactically-valid <x>.
                    WHEN jsonb_typeof(elem -> 'author') = 'string'
                         AND substring(elem ->> 'author' FROM 1 FOR 7) = 'agents/'
                         AND length(elem ->> 'author') > 7
                         AND substring(elem ->> 'author' FROM 8) !~ '[/[:space:]]'
                        THEN jsonb_build_object(
                            'Agent',
                            jsonb_build_object('name', substring(elem ->> 'author' FROM 8))
                        )
                    -- Bare `<x>` matching a known agent.
                    WHEN jsonb_typeof(elem -> 'author') = 'string'
                         AND (elem ->> 'author') <> ''
                         AND (elem ->> 'author') !~ '[/[:space:]]'
                         AND EXISTS (
                             SELECT 1 FROM metis.agents
                             WHERE name = elem ->> 'author'
                         )
                        THEN jsonb_build_object(
                            'Agent',
                            jsonb_build_object('name', elem ->> 'author')
                        )
                    -- Bare `<username>` (most pre-Phase-5b reviews).
                    WHEN jsonb_typeof(elem -> 'author') = 'string'
                         AND (elem ->> 'author') <> ''
                         AND (elem ->> 'author') !~ '[/[:space:]]'
                        THEN jsonb_build_object(
                            'User',
                            jsonb_build_object('name', elem ->> 'author')
                        )
                    -- Anything else: leave as-is; runtime deserializer
                    -- will warn and fall through `parse_legacy_assignee`.
                    ELSE elem -> 'author'
                END,
                'submitted_at', elem -> 'submitted_at'
            )
            ORDER BY ord
        )
        FROM jsonb_array_elements(p.reviews) WITH ORDINALITY AS arr(elem, ord)
    ),
    p.reviews
)
WHERE p.reviews IS NOT NULL
  AND jsonb_typeof(p.reviews) = 'array'
  AND jsonb_array_length(p.reviews) > 0;
