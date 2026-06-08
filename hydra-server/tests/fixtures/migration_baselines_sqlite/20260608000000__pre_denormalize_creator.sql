-- baseline-version: 20260608000000
-- SQLite pre-denormalize-creator baseline. INSERTs are valid against
-- the schema state at sqlite migration
-- `20260608000000_drop_status_icon.sql`, immediately before
-- `20260609000000_add_creator_to_auth_tokens.sql` denormalizes
-- `auth_tokens.creator` and `20260609010000_drop_actors_v2.sql`
-- removes the `actors_v2` table.
--
-- Scope: per [[i-lnmjbjxk]], the creator-denormalize migration needs
-- migration-framework coverage on both backfill paths plus the
-- subsequent `DROP TABLE actors_v2`:
--   * session-bound token whose `session_id` joins to a `tasks_v2`
--     row with a distinguishing `creator` ('alice'). Asserts the
--     `tasks_v2.creator` backfill path.
--   * non-session token whose `actor_name = 'users/bob'`. Asserts
--     the `substr(actor_name, length('users/') + 1)` backfill path.
--   * an `actors_v2` row that must compile against the pre-drop
--     schema and disappear after `DROP TABLE actors_v2` runs.

-- ------------------------------------------------------------
-- 1. tasks_v2 row for the session-bound token's backfill source.
-- ------------------------------------------------------------
INSERT INTO tasks_v2 (
    id, version_number, creator, status, deleted, is_latest,
    spawned_from, conversation_id,
    mount_spec, agent_config, mode,
    actor, creation_time
)
VALUES (
    's-alicexx', 1, 'alice', 'complete', 0, 1,
    NULL, NULL,
    '{"working_dir":"repo","mounts":[]}',
    '{}',
    '{"type":"headless"}',
    NULL,
    '2026-06-08T10:30:00Z'
);

-- ------------------------------------------------------------
-- 2. auth_tokens rows the new migration must backfill.
-- ------------------------------------------------------------
-- Session-bound token. The session-source backfill must copy
-- creator='alice' off tasks_v2.s-alicexx.
INSERT INTO auth_tokens (actor_name, token_hash, session_id, is_revoked)
VALUES ('agents/swe', 'hash-session-alice', 's-alicexx', 0);

-- Non-session-scoped token minted via `setup_local_auth` /
-- `login_with_github_token`. The username-parse backfill must copy
-- creator='bob' off the `users/bob` actor_name.
INSERT INTO auth_tokens (actor_name, token_hash, session_id, is_revoked)
VALUES ('users/bob', 'hash-cli-bob', NULL, 0);

-- ------------------------------------------------------------
-- 3. actors_v2 row that exercises the `DROP TABLE` step.
-- ------------------------------------------------------------
INSERT INTO actors_v2 (
    id,
    version_number,
    auth_token_hash,
    auth_token_salt,
    actor_id,
    creator,
    actor,
    is_latest
)
VALUES (
    'agents/swe',
    1,
    '',
    '',
    '{"Agent":"swe"}',
    'alice',
    NULL,
    1
);
