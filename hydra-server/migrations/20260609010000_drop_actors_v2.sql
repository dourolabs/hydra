-- `actors_v2` was the source of the patch-author attribution bug: every
-- agent session for a given `agents/<name>` shared a single row whose
-- `creator` column was pinned to the first user that instantiated it.
-- The previous migration in this PR denormalized `creator` onto
-- `auth_tokens`, so the table is now write-only dead weight. Drop it.

DROP TRIGGER IF EXISTS set_timestamp_actors_v2 ON metis.actors_v2;
DROP TRIGGER IF EXISTS trg_maintain_latest_actors_v2 ON metis.actors_v2;
DROP TABLE IF EXISTS metis.actors_v2;
