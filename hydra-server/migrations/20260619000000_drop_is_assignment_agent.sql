-- Drop the `is_assignment_agent` column from the agents table.
-- The assignment-agent concept has been removed; per-status
-- `on_enter.assign_to` (`hydra-common::api::v1::projects::StatusOnEnter`)
-- is now the canonical mechanism for routing issues to an agent.
--
-- The partial unique index `agents_assignment_idx` that previously enforced
-- "at most one non-deleted assignment agent" was already dropped by
-- `20260605010000_drop_agent_role_uniqueness_indexes.sql`, so no extra
-- index cleanup is needed here.
ALTER TABLE metis.agents DROP COLUMN is_assignment_agent;
