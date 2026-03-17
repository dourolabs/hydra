-- Fix creator column for user actors in actors_v2.
-- The backfill migration 20260220000000_backfill_null_creators.sql set creator = 'unknown'
-- for all actors. For Username actors, creator should be the plain username string
-- (matching the invariant enforced by Actor::new_for_user).
-- This migration is idempotent: it only updates rows where creator is still 'unknown'.

UPDATE hydra.actors_v2
SET creator = actor_id ->> 'Username'
WHERE actor_id ? 'Username'
  AND creator = 'unknown';
