-- Rewrite remaining "Task" actor variants to "Session" in JSON columns.
-- The previous migration (20260314000000) only caught records with "t-" prefixed IDs.
-- Records created during the transition window have "Task":"s-xxx" which were missed.
-- This migration is idempotent: REPLACE on values without "Task" is a no-op.

-- JSON-serialized ActorId in actors_v2.actor_id:
UPDATE actors_v2 SET actor_id = REPLACE(actor_id, '"Task":', '"Session":') WHERE actor_id LIKE '%"Task"%';

-- Actor columns across all versioned tables (ActorRef JSON with nested ActorId):
UPDATE repositories_v2 SET actor = REPLACE(actor, '"Task":', '"Session":') WHERE actor LIKE '%"Task"%';
UPDATE actors_v2 SET actor = REPLACE(actor, '"Task":', '"Session":') WHERE actor LIKE '%"Task"%';
UPDATE users_v2 SET actor = REPLACE(actor, '"Task":', '"Session":') WHERE actor LIKE '%"Task"%';
UPDATE issues_v2 SET actor = REPLACE(actor, '"Task":', '"Session":') WHERE actor LIKE '%"Task"%';
UPDATE patches_v2 SET actor = REPLACE(actor, '"Task":', '"Session":') WHERE actor LIKE '%"Task"%';
UPDATE tasks_v2 SET actor = REPLACE(actor, '"Task":', '"Session":') WHERE actor LIKE '%"Task"%';
UPDATE documents_v2 SET actor = REPLACE(actor, '"Task":', '"Session":') WHERE actor LIKE '%"Task"%';
UPDATE messages_v2 SET actor = REPLACE(actor, '"Task":', '"Session":') WHERE actor LIKE '%"Task"%';
