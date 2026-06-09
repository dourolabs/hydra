-- Sister to the Postgres `20260613010000_add_issues_v2_status_sequence.sql`.
-- Adds `issues_v2.status_sequence` and backfills it from
-- `(project_id, status)` by joining against the new `statuses` table.
-- See [[i-bqimglba]] for design.
--
-- SQLite differences vs. Postgres:
--   * No `metis.` schema prefix.
--   * BIGINT becomes INTEGER (SQLite collapses both to a 64-bit int).
--   * No `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` in SQLite — the
--     migration tracker prevents the body from running twice in
--     production, and the migration-framework test re-runs through
--     the tracker (which is a no-op) rather than re-executing this
--     body verbatim.
--   * No `UPDATE ... FROM` in older SQLite; this body uses a
--     correlated sub-SELECT, which works on every supported SQLite
--     version.
--   * SQLite has no `RAISE EXCEPTION` outside of trigger bodies, so
--     the pre-flight NULL guard coerces `null_count > 0` into a CHECK
--     constraint violation via a one-row scratch table.

ALTER TABLE issues_v2 ADD COLUMN status_sequence INTEGER;

-- Idempotent backfill: only fills NULL rows, so re-running this body
-- against an already-backfilled table is a no-op. The correlated
-- sub-SELECT returns NULL when no `(project_id, status)` match
-- exists in `statuses`, which the pre-flight guard below catches.
UPDATE issues_v2
   SET status_sequence = (
       SELECT s.sequence
         FROM statuses s
        WHERE s.project_id = issues_v2.project_id
          AND s.key        = issues_v2.status
   )
 WHERE status_sequence IS NULL;

CREATE INDEX IF NOT EXISTS issues_v2_project_status_sequence_idx
    ON issues_v2 (project_id, status_sequence);

-- Pre-flight NULL guard. SQLite cannot RAISE outside of triggers, so
-- count remaining NULL rows into a scratch table whose CHECK
-- constraint trips on any non-zero count. `DROP TABLE IF EXISTS` at
-- the head clears any leftover from a previously-aborted migration so
-- the CREATE always sees a fresh table. The trailing DROP TABLE cleans
-- the scratch on the success path (a CHECK failure inside the
-- migration transaction would roll the CREATE back too).
DROP TABLE IF EXISTS _status_sequence_null_guard;
CREATE TABLE _status_sequence_null_guard (
    null_count INTEGER NOT NULL CHECK (null_count = 0)
);
INSERT INTO _status_sequence_null_guard (null_count)
    SELECT COUNT(*) FROM issues_v2 WHERE status_sequence IS NULL;
DROP TABLE _status_sequence_null_guard;
