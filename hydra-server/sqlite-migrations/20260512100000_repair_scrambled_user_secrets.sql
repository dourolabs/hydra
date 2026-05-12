-- Repair user_secrets rows scrambled by the 20260330000000_user_secrets_composite_pk
-- migration on SQLite. That migration did `INSERT INTO user_secrets_new SELECT * FROM
-- user_secrets`, which is positional. The source table (after
-- 20260316000000_add_internal_to_user_secrets) had column order
-- (username, secret_name, encrypted_value, created_at, updated_at, internal), but the
-- destination put `internal` ahead of the timestamps, so positions 4-6 got cross-mapped:
--   * `internal`   <- old `created_at` (ISO timestamp stored as TEXT)
--   * `created_at` <- old `updated_at` (ISO timestamp stored as TEXT)
--   * `updated_at` <- old `internal`   (0 or 1, stored as "0"/"1" via TEXT affinity)
-- `encrypted_value` (position 3) is unaffected.
--
-- Scrambled rows are detected via `updated_at IN ('0', '1')` — real `updated_at` is
-- always an ISO timestamp, so those values can only arise from the column-reorder bug.
-- The repair is idempotent: on a database that never had scrambled rows, the scrambled
-- branch matches nothing and every row is copied verbatim through the non-scrambled
-- branch.
--
-- We rebuild the table using an *explicit* column list in `INSERT ... SELECT`, which is
-- the pattern the original migration should have used and prevents this class of bug
-- from recurring here.
--
-- Subsequent `set_user_secret` calls on a scrambled row inserted a second "zombie" row
-- because `ON CONFLICT (username, secret_name, internal)` did not match the corrupted
-- PK. Inserting non-scrambled rows first and using `INSERT OR IGNORE` for the recovered
-- scrambled rows dedupes those pairs, preferring the zombie (which has the user's
-- newer value).

CREATE TABLE user_secrets_repaired (
    username TEXT NOT NULL,
    secret_name TEXT NOT NULL,
    encrypted_value BLOB NOT NULL,
    internal BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    PRIMARY KEY (username, secret_name, internal)
);

INSERT OR IGNORE INTO user_secrets_repaired
    (username, secret_name, encrypted_value, internal, created_at, updated_at)
SELECT username, secret_name, encrypted_value, internal, created_at, updated_at
FROM user_secrets
WHERE updated_at NOT IN ('0', '1');

INSERT OR IGNORE INTO user_secrets_repaired
    (username, secret_name, encrypted_value, internal, created_at, updated_at)
SELECT
    username,
    secret_name,
    encrypted_value,
    CAST(updated_at AS INTEGER) AS internal,
    internal AS created_at,
    created_at AS updated_at
FROM user_secrets
WHERE updated_at IN ('0', '1');

DROP TABLE user_secrets;
ALTER TABLE user_secrets_repaired RENAME TO user_secrets;
