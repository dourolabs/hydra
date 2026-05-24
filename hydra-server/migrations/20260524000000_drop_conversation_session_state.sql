-- Drop the legacy conversation_session_state table.
-- Phase E step 19 of designs/sessions-orthogonality-redesign.md.
-- All rows were migrated to metis.session_state_v2 (keyed on the producing
-- session id) by `hydra-migrate-sessions migrate-state` during Phase C step 9.

DROP TABLE IF EXISTS metis.conversation_session_state;
