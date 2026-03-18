-- Add denormalized timing columns to tasks_v2.
ALTER TABLE tasks_v2 ADD COLUMN creation_time TEXT;
ALTER TABLE tasks_v2 ADD COLUMN start_time TEXT;
ALTER TABLE tasks_v2 ADD COLUMN end_time TEXT;

-- Backfill timing on the latest version of each task.
-- creation_time: created_at of version 1.
UPDATE tasks_v2
SET creation_time = (
    SELECT created_at FROM tasks_v2 t2
    WHERE t2.id = tasks_v2.id AND t2.version_number = 1
)
WHERE creation_time IS NULL
  AND version_number = (
      SELECT MAX(version_number) FROM tasks_v2 t3 WHERE t3.id = tasks_v2.id
  );

-- start_time: earliest created_at where status = 'running'.
UPDATE tasks_v2
SET start_time = (
    SELECT MIN(created_at) FROM tasks_v2 t2
    WHERE t2.id = tasks_v2.id AND t2.status = 'running'
)
WHERE start_time IS NULL
  AND version_number = (
      SELECT MAX(version_number) FROM tasks_v2 t3 WHERE t3.id = tasks_v2.id
  );

-- end_time: earliest created_at where status in ('complete', 'failed').
UPDATE tasks_v2
SET end_time = (
    SELECT MIN(created_at) FROM tasks_v2 t2
    WHERE t2.id = tasks_v2.id AND t2.status IN ('complete', 'failed')
)
WHERE end_time IS NULL
  AND version_number = (
      SELECT MAX(version_number) FROM tasks_v2 t3 WHERE t3.id = tasks_v2.id
  );
