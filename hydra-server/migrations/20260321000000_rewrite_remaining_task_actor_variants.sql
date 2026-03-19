-- Rewrite remaining "Task" actor variants to "Session" in JSON columns.
-- The previous migration (20260316000000) only caught records with "t-" prefixed IDs.
-- Records created during the transition window have "Task":"s-xxx" which were missed.
-- This migration is idempotent: REPLACE on values without "Task" is a no-op.

-- JSON-serialized ActorId in actors_v2.actor_id:
UPDATE hydra.actors_v2 SET actor_id = REPLACE(actor_id::TEXT, '"Task":', '"Session":')::JSONB WHERE actor_id::TEXT LIKE '%"Task"%';

-- Actor columns across all versioned tables (ActorRef JSON with nested ActorId):
UPDATE hydra.repositories_v2 SET actor = REPLACE(actor::TEXT, '"Task":', '"Session":')::JSONB WHERE actor::TEXT LIKE '%"Task"%';
UPDATE hydra.actors_v2 SET actor = REPLACE(actor::TEXT, '"Task":', '"Session":')::JSONB WHERE actor::TEXT LIKE '%"Task"%';
UPDATE hydra.users_v2 SET actor = REPLACE(actor::TEXT, '"Task":', '"Session":')::JSONB WHERE actor::TEXT LIKE '%"Task"%';
UPDATE hydra.issues_v2 SET actor = REPLACE(actor::TEXT, '"Task":', '"Session":')::JSONB WHERE actor::TEXT LIKE '%"Task"%';
UPDATE hydra.patches_v2 SET actor = REPLACE(actor::TEXT, '"Task":', '"Session":')::JSONB WHERE actor::TEXT LIKE '%"Task"%';
UPDATE hydra.tasks_v2 SET actor = REPLACE(actor::TEXT, '"Task":', '"Session":')::JSONB WHERE actor::TEXT LIKE '%"Task"%';
UPDATE hydra.documents_v2 SET actor = REPLACE(actor::TEXT, '"Task":', '"Session":')::JSONB WHERE actor::TEXT LIKE '%"Task"%';
UPDATE hydra.messages_v2 SET actor = REPLACE(actor::TEXT, '"Task":', '"Session":')::JSONB WHERE actor::TEXT LIKE '%"Task"%';
