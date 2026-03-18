-- Rewrite all legacy t-prefixed session IDs to use the s- prefix.
-- This migration is idempotent: REPLACE on values that already have s- is a no-op.

-- Direct ID columns: replace t- prefix with s-
UPDATE tasks_v2 SET id = 's-' || SUBSTR(id, 3) WHERE id LIKE 't-%';
UPDATE notifications SET object_id = 's-' || SUBSTR(object_id, 3) WHERE object_id LIKE 't-%';
UPDATE label_associations SET object_id = 's-' || SUBSTR(object_id, 3) WHERE object_id LIKE 't-%';

-- ActorId Display format columns: replace "w-t-" with "w-s-"
UPDATE notifications SET recipient = REPLACE(recipient, 'w-t-', 'w-s-') WHERE recipient LIKE '%w-t-%';
UPDATE notifications SET source_actor = REPLACE(source_actor, 'w-t-', 'w-s-') WHERE source_actor LIKE '%w-t-%';

-- JSON-serialized ActorId in actors_v2.actor_id:
-- Normalize "Task":"t-xxx" -> "Session":"s-xxx" and "Session":"t-xxx" -> "Session":"s-xxx"
UPDATE actors_v2 SET actor_id = REPLACE(REPLACE(actor_id, '"Task":"t-', '"Session":"s-'), '"Session":"t-', '"Session":"s-') WHERE actor_id LIKE '%"t-%';

-- Actor columns across versioned tables (ActorRef JSON with nested ActorId):
-- Apply same replacements for embedded session IDs in JSON
UPDATE repositories_v2 SET actor = REPLACE(REPLACE(actor, '"Task":"t-', '"Session":"s-'), '"Session":"t-', '"Session":"s-') WHERE actor LIKE '%"t-%';
UPDATE actors_v2 SET actor = REPLACE(REPLACE(actor, '"Task":"t-', '"Session":"s-'), '"Session":"t-', '"Session":"s-') WHERE actor LIKE '%"t-%';
UPDATE users_v2 SET actor = REPLACE(REPLACE(actor, '"Task":"t-', '"Session":"s-'), '"Session":"t-', '"Session":"s-') WHERE actor LIKE '%"t-%';
UPDATE issues_v2 SET actor = REPLACE(REPLACE(actor, '"Task":"t-', '"Session":"s-'), '"Session":"t-', '"Session":"s-') WHERE actor LIKE '%"t-%';
UPDATE patches_v2 SET actor = REPLACE(REPLACE(actor, '"Task":"t-', '"Session":"s-'), '"Session":"t-', '"Session":"s-') WHERE actor LIKE '%"t-%';
UPDATE tasks_v2 SET actor = REPLACE(REPLACE(actor, '"Task":"t-', '"Session":"s-'), '"Session":"t-', '"Session":"s-') WHERE actor LIKE '%"t-%';
UPDATE documents_v2 SET actor = REPLACE(REPLACE(actor, '"Task":"t-', '"Session":"s-'), '"Session":"t-', '"Session":"s-') WHERE actor LIKE '%"t-%';
UPDATE messages_v2 SET actor = REPLACE(REPLACE(actor, '"Task":"t-', '"Session":"s-'), '"Session":"t-', '"Session":"s-') WHERE actor LIKE '%"t-%';

-- Text fields containing ID references in notifications.summary:
-- Replace "t-" followed by lowercase letters (session ID pattern) with "s-" prefix.
-- SQLite lacks regex replace, so we use a targeted approach for the known pattern "Job t-" -> "Job s-"
-- and the general pattern via REPLACE for the common "t-" occurrences in summary text.
UPDATE notifications SET summary = REPLACE(summary, ' t-', ' s-') WHERE summary LIKE '% t-%';
