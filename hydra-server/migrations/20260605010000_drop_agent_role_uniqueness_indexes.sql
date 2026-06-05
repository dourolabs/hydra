-- Agent role-flag uniqueness (`is_assignment_agent`,
-- `is_default_conversation_agent`) is workflow state, not referential
-- integrity, and is now enforced by the `agent_role_uniqueness` `Restriction`
-- in `AppState` (see `docs/architecture/domain-store-routes.md`). Drop the
-- partial unique indexes that previously enforced the rule at the schema
-- layer so the store no longer double-enforces it.
DROP INDEX IF EXISTS metis.agents_assignment_idx;
DROP INDEX IF EXISTS metis.agents_default_conversation_idx;
