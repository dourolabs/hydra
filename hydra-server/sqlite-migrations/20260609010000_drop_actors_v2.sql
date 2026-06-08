-- `actors_v2` was the source of the patch-author attribution bug: every
-- agent session for a given `agents/<name>` shared a single row whose
-- `creator` column was pinned to the first user that instantiated it.
-- The previous migration in this PR denormalized `creator` onto
-- `auth_tokens`, so the table is now write-only dead weight. Drop it.

DROP INDEX IF EXISTS actors_v2_latest_idx;
DROP INDEX IF EXISTS actors_v2_latest_id_idx;
DROP INDEX IF EXISTS actors_v2_latest_pagination_idx;
DROP TABLE IF EXISTS actors_v2;
