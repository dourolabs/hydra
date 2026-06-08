-- Denormalize the originating creator onto `auth_tokens` so request-time
-- auth resolves a token's `creator` in a single lookup, removing the
-- per-actor `actors_v2` round-trip that drove the patch-author bug
-- (multiple agents sharing one actor row were all attributed to the
-- first user who instantiated it). The follow-on migration in this PR
-- drops `actors_v2`.
--
-- Two backfill sources cover every existing row:
--   * `session_id IS NOT NULL` (session-spawned tokens) → the matching
--     `tasks_v2.creator` of the latest task version.
--   * `session_id IS NULL` (user CLI tokens minted by `setup_local_auth`
--     / `login_with_github_token`) → the username embedded in
--     `actor_name = 'users/<username>'`.
--
-- A defensive `CHECK (creator <> '__backfill__')` defends against any
-- future INSERT path forgetting to populate `creator`.

ALTER TABLE auth_tokens ADD COLUMN creator TEXT NOT NULL DEFAULT '__backfill__';

UPDATE auth_tokens
SET creator = (
    SELECT t.creator
    FROM tasks_v2 t
    WHERE t.id = auth_tokens.session_id AND t.is_latest = 1
)
WHERE session_id IS NOT NULL
  AND EXISTS (
      SELECT 1 FROM tasks_v2 t
      WHERE t.id = auth_tokens.session_id AND t.is_latest = 1
  );

UPDATE auth_tokens
SET creator = substr(actor_name, length('users/') + 1)
WHERE session_id IS NULL
  AND actor_name LIKE 'users/%';

-- Catch-all for any leftover '__backfill__' rows (session_id pointed at a
-- vanished task, or actor_name was not a 'users/<name>' shape). These
-- shouldn't exist in well-formed data but we still need a non-default
-- value before tightening the column.
UPDATE auth_tokens SET creator = 'unknown' WHERE creator = '__backfill__';

-- SQLite has no `ALTER COLUMN ... DROP DEFAULT`; rebuild via the
-- create-new / copy / rename dance. List every column explicitly in both
-- INSERT and SELECT to avoid positional drift.
CREATE TABLE auth_tokens_new (
    actor_name TEXT NOT NULL,
    token_hash TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    session_id TEXT,
    is_revoked INTEGER NOT NULL DEFAULT 0,
    creator TEXT NOT NULL CHECK (creator <> '__backfill__'),
    PRIMARY KEY (actor_name, token_hash)
);

INSERT INTO auth_tokens_new (
    actor_name,
    token_hash,
    created_at,
    session_id,
    is_revoked,
    creator
)
SELECT
    actor_name,
    token_hash,
    created_at,
    session_id,
    is_revoked,
    creator
FROM auth_tokens;

DROP TABLE auth_tokens;
ALTER TABLE auth_tokens_new RENAME TO auth_tokens;

CREATE INDEX IF NOT EXISTS auth_tokens_actor_name_idx ON auth_tokens (actor_name);
CREATE INDEX IF NOT EXISTS auth_tokens_session_id_idx ON auth_tokens (session_id);
