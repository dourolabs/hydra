-- Backfill any remaining NULL creator rows (idempotent — previous backfills
-- in 20260220 / 20260319 should have caught everything, but defend against
-- drift) and then enforce NOT NULL on the creator column for all three
-- affected v2 tables.
UPDATE metis.actors_v2 SET creator = 'unknown' WHERE creator IS NULL;
UPDATE metis.tasks_v2 SET creator = 'unknown' WHERE creator IS NULL;
UPDATE metis.patches_v2 SET creator = 'unknown' WHERE creator IS NULL;

ALTER TABLE metis.actors_v2 ALTER COLUMN creator SET NOT NULL;
ALTER TABLE metis.tasks_v2 ALTER COLUMN creator SET NOT NULL;
ALTER TABLE metis.patches_v2 ALTER COLUMN creator SET NOT NULL;
