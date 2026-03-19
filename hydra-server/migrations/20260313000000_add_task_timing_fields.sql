-- Add denormalized timing columns to tasks_v2 so that list_tasks() can read
-- them directly instead of computing from version history.

ALTER TABLE metis.tasks_v2 ADD COLUMN IF NOT EXISTS creation_time TIMESTAMPTZ;
ALTER TABLE metis.tasks_v2 ADD COLUMN IF NOT EXISTS start_time TIMESTAMPTZ;
ALTER TABLE metis.tasks_v2 ADD COLUMN IF NOT EXISTS end_time TIMESTAMPTZ;

-- Backfill timing on the latest version of each task.
-- creation_time: timestamp of the first version (version_number = 1).
UPDATE metis.tasks_v2 t
SET creation_time = sub.ct
FROM (
    SELECT id, created_at AS ct
    FROM metis.tasks_v2
    WHERE version_number = 1
) sub
WHERE t.id = sub.id
  AND t.creation_time IS NULL
  AND t.version_number = (
      SELECT MAX(version_number) FROM metis.tasks_v2 WHERE id = t.id
  );

-- start_time: created_at of the earliest version with status = 'running'.
UPDATE metis.tasks_v2 t
SET start_time = sub.st
FROM (
    SELECT id, MIN(created_at) AS st
    FROM metis.tasks_v2
    WHERE status = 'running'
    GROUP BY id
) sub
WHERE t.id = sub.id
  AND t.start_time IS NULL
  AND t.version_number = (
      SELECT MAX(version_number) FROM metis.tasks_v2 WHERE id = t.id
  );

-- end_time: created_at of the earliest version with status in ('complete', 'failed').
UPDATE metis.tasks_v2 t
SET end_time = sub.et
FROM (
    SELECT id, MIN(created_at) AS et
    FROM metis.tasks_v2
    WHERE status IN ('complete', 'failed')
    GROUP BY id
) sub
WHERE t.id = sub.id
  AND t.end_time IS NULL
  AND t.version_number = (
      SELECT MAX(version_number) FROM metis.tasks_v2 WHERE id = t.id
  );
