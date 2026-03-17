-- Final catch-all migration to rewrite ALL remaining t- prefixed session IDs to s-.
-- Previous migrations (20260316, 20260320, 20260321) missed some records, likely
-- created during transition windows or with JSON formatting variants not caught by
-- the specific REPLACE patterns used.
-- This migration is idempotent: safe to re-run on already-fixed databases.

-- ============================================================
-- 1. Direct ID columns: replace t- prefix with s-
-- ============================================================
UPDATE hydra.tasks_v2 SET id = 's-' || SUBSTRING(id FROM 3) WHERE id LIKE 't-%';
UPDATE hydra.notifications SET object_id = 's-' || SUBSTRING(object_id FROM 3) WHERE object_id LIKE 't-%';
UPDATE hydra.label_associations SET object_id = 's-' || SUBSTRING(object_id FROM 3) WHERE object_id LIKE 't-%';

-- ============================================================
-- 2. created_by columns: replace t- prefix with s-
-- ============================================================
UPDATE hydra.patches_v2 SET created_by = 's-' || SUBSTRING(created_by FROM 3) WHERE created_by LIKE 't-%';
UPDATE hydra.documents_v2 SET created_by = 's-' || SUBSTRING(created_by FROM 3) WHERE created_by LIKE 't-%';

-- ============================================================
-- 3. ActorId Display format columns (notifications): replace w-t- with w-s-
-- ============================================================
UPDATE hydra.notifications SET recipient = REPLACE(recipient, 'w-t-', 'w-s-') WHERE recipient LIKE '%w-t-%';
UPDATE hydra.notifications SET source_actor = REPLACE(source_actor, 'w-t-', 'w-s-') WHERE source_actor LIKE '%w-t-%';

-- ============================================================
-- 4. JSON-serialized ActorId in actors_v2.actor_id:
--    Catch both "Task":"t-xxx" and "Session":"t-xxx" variants,
--    as well as any remaining "Task":"s-xxx" variants.
-- ============================================================
UPDATE hydra.actors_v2
  SET actor_id = REPLACE(REPLACE(REPLACE(actor_id::TEXT,
    '"Task":"t-', '"Session":"s-'),
    '"Session":"t-', '"Session":"s-'),
    '"Task":', '"Session":')::JSONB
  WHERE actor_id::TEXT LIKE '%"t-%' OR actor_id::TEXT LIKE '%"Task"%';

-- ============================================================
-- 5. Actor columns across all versioned tables:
--    Apply all three replacements in one pass:
--    a) "Task":"t-xxx" -> "Session":"s-xxx"
--    b) "Session":"t-xxx" -> "Session":"s-xxx"
--    c) "Task":"s-xxx" -> "Session":"s-xxx" (leftover variant key)
-- ============================================================
UPDATE hydra.repositories_v2
  SET actor = REPLACE(REPLACE(REPLACE(actor::TEXT,
    '"Task":"t-', '"Session":"s-'),
    '"Session":"t-', '"Session":"s-'),
    '"Task":', '"Session":')::JSONB
  WHERE actor::TEXT LIKE '%"t-%' OR actor::TEXT LIKE '%"Task"%';

UPDATE hydra.actors_v2
  SET actor = REPLACE(REPLACE(REPLACE(actor::TEXT,
    '"Task":"t-', '"Session":"s-'),
    '"Session":"t-', '"Session":"s-'),
    '"Task":', '"Session":')::JSONB
  WHERE actor::TEXT LIKE '%"t-%' OR actor::TEXT LIKE '%"Task"%';

UPDATE hydra.users_v2
  SET actor = REPLACE(REPLACE(REPLACE(actor::TEXT,
    '"Task":"t-', '"Session":"s-'),
    '"Session":"t-', '"Session":"s-'),
    '"Task":', '"Session":')::JSONB
  WHERE actor::TEXT LIKE '%"t-%' OR actor::TEXT LIKE '%"Task"%';

UPDATE hydra.issues_v2
  SET actor = REPLACE(REPLACE(REPLACE(actor::TEXT,
    '"Task":"t-', '"Session":"s-'),
    '"Session":"t-', '"Session":"s-'),
    '"Task":', '"Session":')::JSONB
  WHERE actor::TEXT LIKE '%"t-%' OR actor::TEXT LIKE '%"Task"%';

UPDATE hydra.patches_v2
  SET actor = REPLACE(REPLACE(REPLACE(actor::TEXT,
    '"Task":"t-', '"Session":"s-'),
    '"Session":"t-', '"Session":"s-'),
    '"Task":', '"Session":')::JSONB
  WHERE actor::TEXT LIKE '%"t-%' OR actor::TEXT LIKE '%"Task"%';

UPDATE hydra.tasks_v2
  SET actor = REPLACE(REPLACE(REPLACE(actor::TEXT,
    '"Task":"t-', '"Session":"s-'),
    '"Session":"t-', '"Session":"s-'),
    '"Task":', '"Session":')::JSONB
  WHERE actor::TEXT LIKE '%"t-%' OR actor::TEXT LIKE '%"Task"%';

UPDATE hydra.documents_v2
  SET actor = REPLACE(REPLACE(REPLACE(actor::TEXT,
    '"Task":"t-', '"Session":"s-'),
    '"Session":"t-', '"Session":"s-'),
    '"Task":', '"Session":')::JSONB
  WHERE actor::TEXT LIKE '%"t-%' OR actor::TEXT LIKE '%"Task"%';

UPDATE hydra.messages_v2
  SET actor = REPLACE(REPLACE(REPLACE(actor::TEXT,
    '"Task":"t-', '"Session":"s-'),
    '"Session":"t-', '"Session":"s-'),
    '"Task":', '"Session":')::JSONB
  WHERE actor::TEXT LIKE '%"t-%' OR actor::TEXT LIKE '%"Task"%';

-- ============================================================
-- 6. Text fields: notifications.summary
--    Use regexp_replace to catch any remaining t- prefixed IDs
-- ============================================================
UPDATE hydra.notifications SET summary = regexp_replace(summary, '\mt-([a-z]+)', 's-\1', 'g') WHERE summary ~ '\mt-[a-z]+';
