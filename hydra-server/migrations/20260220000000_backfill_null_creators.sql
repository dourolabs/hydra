-- Backfill NULL creator values to 'unknown' across all three affected tables.
-- This supports making creator mandatory in the application layer.
-- The column remains nullable for now; a NOT NULL constraint will be added
-- after all domain types (Actor, Task, Patch) have been migrated.
UPDATE hydra.actors_v2 SET creator = 'unknown' WHERE creator IS NULL;
UPDATE hydra.tasks_v2 SET creator = 'unknown' WHERE creator IS NULL;
UPDATE hydra.patches_v2 SET creator = 'unknown' WHERE creator IS NULL;
