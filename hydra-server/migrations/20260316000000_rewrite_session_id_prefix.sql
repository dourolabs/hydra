-- Rewrite all legacy t-prefixed session IDs to use the s- prefix.
-- This migration is idempotent: replacements on values that already have s- are no-ops.

-- Direct ID columns: replace t- prefix with s-
UPDATE hydra.tasks_v2 SET id = 's-' || SUBSTRING(id FROM 3) WHERE id LIKE 't-%';
UPDATE hydra.notifications SET object_id = 's-' || SUBSTRING(object_id FROM 3) WHERE object_id LIKE 't-%';
UPDATE hydra.label_associations SET object_id = 's-' || SUBSTRING(object_id FROM 3) WHERE object_id LIKE 't-%';

-- ActorId Display format columns: replace "w-t-" with "w-s-"
UPDATE hydra.notifications SET recipient = REPLACE(recipient, 'w-t-', 'w-s-') WHERE recipient LIKE '%w-t-%';
UPDATE hydra.notifications SET source_actor = REPLACE(source_actor, 'w-t-', 'w-s-') WHERE source_actor LIKE '%w-t-%';

-- JSON-serialized ActorId in actors_v2.actor_id:
-- Normalize "Task":"t-xxx" -> "Session":"s-xxx" and "Session":"t-xxx" -> "Session":"s-xxx"
UPDATE hydra.actors_v2 SET actor_id = REPLACE(REPLACE(actor_id::TEXT, '"Task":"t-', '"Session":"s-'), '"Session":"t-', '"Session":"s-')::JSONB WHERE actor_id::TEXT LIKE '%"t-%';

-- Actor columns across versioned tables (ActorRef JSON with nested ActorId):
-- Apply same replacements for embedded session IDs in JSON
UPDATE hydra.repositories_v2 SET actor = REPLACE(REPLACE(actor::TEXT, '"Task":"t-', '"Session":"s-'), '"Session":"t-', '"Session":"s-')::JSONB WHERE actor::TEXT LIKE '%"t-%';
UPDATE hydra.actors_v2 SET actor = REPLACE(REPLACE(actor::TEXT, '"Task":"t-', '"Session":"s-'), '"Session":"t-', '"Session":"s-')::JSONB WHERE actor::TEXT LIKE '%"t-%';
UPDATE hydra.users_v2 SET actor = REPLACE(REPLACE(actor::TEXT, '"Task":"t-', '"Session":"s-'), '"Session":"t-', '"Session":"s-')::JSONB WHERE actor::TEXT LIKE '%"t-%';
UPDATE hydra.issues_v2 SET actor = REPLACE(REPLACE(actor::TEXT, '"Task":"t-', '"Session":"s-'), '"Session":"t-', '"Session":"s-')::JSONB WHERE actor::TEXT LIKE '%"t-%';
UPDATE hydra.patches_v2 SET actor = REPLACE(REPLACE(actor::TEXT, '"Task":"t-', '"Session":"s-'), '"Session":"t-', '"Session":"s-')::JSONB WHERE actor::TEXT LIKE '%"t-%';
UPDATE hydra.tasks_v2 SET actor = REPLACE(REPLACE(actor::TEXT, '"Task":"t-', '"Session":"s-'), '"Session":"t-', '"Session":"s-')::JSONB WHERE actor::TEXT LIKE '%"t-%';
UPDATE hydra.documents_v2 SET actor = REPLACE(REPLACE(actor::TEXT, '"Task":"t-', '"Session":"s-'), '"Session":"t-', '"Session":"s-')::JSONB WHERE actor::TEXT LIKE '%"t-%';
UPDATE hydra.messages_v2 SET actor = REPLACE(REPLACE(actor::TEXT, '"Task":"t-', '"Session":"s-'), '"Session":"t-', '"Session":"s-')::JSONB WHERE actor::TEXT LIKE '%"t-%';

-- Text fields containing ID references in notifications.summary:
-- Replace session ID patterns like "Job t-abcdef" -> "Job s-abcdef" using regexp_replace.
UPDATE hydra.notifications SET summary = regexp_replace(summary, '\mt-([a-z]+)', 's-\1', 'g') WHERE summary ~ '\mt-[a-z]+';
