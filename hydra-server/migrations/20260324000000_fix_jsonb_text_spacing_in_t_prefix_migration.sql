-- Fix JSONB text spacing in t-prefix migration REPLACE patterns.
-- Previous migrations (20260316, 20260320, 20260321, 20260323) all used REPLACE
-- patterns without a space after the colon (e.g. '"Task":"t-'), but Postgres
-- JSONB::TEXT canonical format inserts a space after colons (e.g. '"Task": "t-').
-- This made all JSONB REPLACE operations no-ops.
-- This migration uses the correct spacing to actually match and fix the data.
-- Idempotent: safe to re-run on already-fixed databases.

-- ============================================================
-- 1. actors_v2.actor_id (JSONB)
-- ============================================================
UPDATE hydra.actors_v2
  SET actor_id = REPLACE(REPLACE(REPLACE(actor_id::TEXT,
    '"Task": "t-', '"Session": "s-'),
    '"Session": "t-', '"Session": "s-'),
    '"Task":', '"Session":')::JSONB
  WHERE actor_id::TEXT LIKE '%"t-%' OR actor_id::TEXT LIKE '%"Task"%';

-- ============================================================
-- 2. actors_v2.actor (JSONB)
-- ============================================================
UPDATE hydra.actors_v2
  SET actor = REPLACE(REPLACE(REPLACE(actor::TEXT,
    '"Task": "t-', '"Session": "s-'),
    '"Session": "t-', '"Session": "s-'),
    '"Task":', '"Session":')::JSONB
  WHERE actor::TEXT LIKE '%"t-%' OR actor::TEXT LIKE '%"Task"%';

-- ============================================================
-- 3. repositories_v2.actor (JSONB)
-- ============================================================
UPDATE hydra.repositories_v2
  SET actor = REPLACE(REPLACE(REPLACE(actor::TEXT,
    '"Task": "t-', '"Session": "s-'),
    '"Session": "t-', '"Session": "s-'),
    '"Task":', '"Session":')::JSONB
  WHERE actor::TEXT LIKE '%"t-%' OR actor::TEXT LIKE '%"Task"%';

-- ============================================================
-- 4. users_v2.actor (JSONB)
-- ============================================================
UPDATE hydra.users_v2
  SET actor = REPLACE(REPLACE(REPLACE(actor::TEXT,
    '"Task": "t-', '"Session": "s-'),
    '"Session": "t-', '"Session": "s-'),
    '"Task":', '"Session":')::JSONB
  WHERE actor::TEXT LIKE '%"t-%' OR actor::TEXT LIKE '%"Task"%';

-- ============================================================
-- 5. issues_v2.actor (JSONB)
-- ============================================================
UPDATE hydra.issues_v2
  SET actor = REPLACE(REPLACE(REPLACE(actor::TEXT,
    '"Task": "t-', '"Session": "s-'),
    '"Session": "t-', '"Session": "s-'),
    '"Task":', '"Session":')::JSONB
  WHERE actor::TEXT LIKE '%"t-%' OR actor::TEXT LIKE '%"Task"%';

-- ============================================================
-- 6. patches_v2.actor (JSONB)
-- ============================================================
UPDATE hydra.patches_v2
  SET actor = REPLACE(REPLACE(REPLACE(actor::TEXT,
    '"Task": "t-', '"Session": "s-'),
    '"Session": "t-', '"Session": "s-'),
    '"Task":', '"Session":')::JSONB
  WHERE actor::TEXT LIKE '%"t-%' OR actor::TEXT LIKE '%"Task"%';

-- ============================================================
-- 7. tasks_v2.actor (JSONB)
-- ============================================================
UPDATE hydra.tasks_v2
  SET actor = REPLACE(REPLACE(REPLACE(actor::TEXT,
    '"Task": "t-', '"Session": "s-'),
    '"Session": "t-', '"Session": "s-'),
    '"Task":', '"Session":')::JSONB
  WHERE actor::TEXT LIKE '%"t-%' OR actor::TEXT LIKE '%"Task"%';

-- ============================================================
-- 8. documents_v2.actor (JSONB)
-- ============================================================
UPDATE hydra.documents_v2
  SET actor = REPLACE(REPLACE(REPLACE(actor::TEXT,
    '"Task": "t-', '"Session": "s-'),
    '"Session": "t-', '"Session": "s-'),
    '"Task":', '"Session":')::JSONB
  WHERE actor::TEXT LIKE '%"t-%' OR actor::TEXT LIKE '%"Task"%';

-- ============================================================
-- 9. messages_v2.actor (JSONB)
-- ============================================================
UPDATE hydra.messages_v2
  SET actor = REPLACE(REPLACE(REPLACE(actor::TEXT,
    '"Task": "t-', '"Session": "s-'),
    '"Session": "t-', '"Session": "s-'),
    '"Task":', '"Session":')::JSONB
  WHERE actor::TEXT LIKE '%"t-%' OR actor::TEXT LIKE '%"Task"%';
