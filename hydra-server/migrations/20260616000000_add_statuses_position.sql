-- Add the `position` column to `metis.statuses`. This is the wire
-- field that backs drag-to-reorder UI on the per-status CRUD surface:
-- once `UpsertProjectRequest.statuses` is gone, the IssuesBoard reorder
-- flow can no longer round-trip the whole project to persist a new
-- order, so each status row must carry its own sort key.
--
-- Backfill `position = sequence` so the post-migration display order
-- matches today's behavior (statuses are currently read back in
-- `sequence ASC` order). After the cutover, callers update `position`
-- via `PUT /v1/projects/:project_ref/statuses/:status_key` and the
-- store's `update_status` path; `sequence` continues to act as the
-- monotonically non-decreasing storage identity.

ALTER TABLE metis.statuses ADD COLUMN position DOUBLE PRECISION NOT NULL DEFAULT 0;

UPDATE metis.statuses SET position = sequence;
