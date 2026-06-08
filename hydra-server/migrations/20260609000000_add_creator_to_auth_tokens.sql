-- Denormalize the originating creator onto `auth_tokens` so request-time
-- auth resolves a token's `creator` in a single lookup, removing the
-- per-actor `actors_v2` round-trip that drove the patch-author bug
-- (multiple agents sharing one actor row were all attributed to the
-- first user who instantiated it). The follow-on migration in this PR
-- drops `actors_v2`.

ALTER TABLE metis.auth_tokens ADD COLUMN creator TEXT NOT NULL DEFAULT '__backfill__';

-- Session-spawned tokens: copy from `tasks_v2.creator` for the matching
-- latest task version.
UPDATE metis.auth_tokens AS a
SET creator = t.creator
FROM metis.tasks_v2 AS t
WHERE a.session_id IS NOT NULL
  AND t.id = a.session_id
  AND t.is_latest = TRUE;

-- User CLI tokens: `actor_name = 'users/<name>'` is the source of truth
-- for the creator on these non-session-scoped rows.
UPDATE metis.auth_tokens
SET creator = substring(actor_name FROM length('users/') + 1)
WHERE session_id IS NULL
  AND actor_name LIKE 'users/%';

-- Drop stragglers whose creator could not be derived (vanished session id,
-- malformed actor_name). These rows would otherwise need a sentinel value
-- to satisfy the NOT NULL + CHECK below, which we don't allow. Any caller
-- relying on such a row would already be failing for an unrelated reason.
DELETE FROM metis.auth_tokens WHERE creator = '__backfill__';

ALTER TABLE metis.auth_tokens ALTER COLUMN creator DROP DEFAULT;
ALTER TABLE metis.auth_tokens
    ADD CONSTRAINT auth_tokens_creator_not_backfill_chk CHECK (creator <> '__backfill__');
