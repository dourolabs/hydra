-- Rewrite remaining "Task" actor variants to "Session" in JSON columns.
-- The previous migration (20260316000000) only caught records with "t-" prefixed IDs.
-- Records created during the transition window have "Task":"s-xxx" which were missed.
-- This migration is idempotent: REPLACE on values without "Task" is a no-op.

-- JSON-serialized ActorId in actors_v2.actor_id:
UPDATE metis.actors_v2 SET actor_id = REPLACE(actor_id::TEXT, '"Task":', '"Session":')::JSONB WHERE actor_id::TEXT LIKE '%"Task"%';

-- Actor columns across all versioned tables (ActorRef JSON with nested ActorId):
UPDATE metis.repositories_v2 SET actor = REPLACE(actor::TEXT, '"Task":', '"Session":')::JSONB WHERE actor::TEXT LIKE '%"Task"%';
UPDATE metis.actors_v2 SET actor = REPLACE(actor::TEXT, '"Task":', '"Session":')::JSONB WHERE actor::TEXT LIKE '%"Task"%';
UPDATE metis.users_v2 SET actor = REPLACE(actor::TEXT, '"Task":', '"Session":')::JSONB WHERE actor::TEXT LIKE '%"Task"%';
UPDATE metis.issues_v2 SET actor = REPLACE(actor::TEXT, '"Task":', '"Session":')::JSONB WHERE actor::TEXT LIKE '%"Task"%';
UPDATE metis.patches_v2 SET actor = REPLACE(actor::TEXT, '"Task":', '"Session":')::JSONB WHERE actor::TEXT LIKE '%"Task"%';
UPDATE metis.tasks_v2 SET actor = REPLACE(actor::TEXT, '"Task":', '"Session":')::JSONB WHERE actor::TEXT LIKE '%"Task"%';
UPDATE metis.documents_v2 SET actor = REPLACE(actor::TEXT, '"Task":', '"Session":')::JSONB WHERE actor::TEXT LIKE '%"Task"%';
UPDATE metis.messages_v2 SET actor = REPLACE(actor::TEXT, '"Task":', '"Session":')::JSONB WHERE actor::TEXT LIKE '%"Task"%';
