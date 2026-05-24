-- Drop the legacy conversations.session_state column (sqlite analog of
-- metis.conversation_session_state in postgres).
-- Phase E step 19 of designs/sessions-orthogonality-redesign.md.
-- All rows were migrated to session_state (keyed on the producing session id)
-- by `hydra-migrate-sessions migrate-state` during Phase C step 9.

ALTER TABLE conversations DROP COLUMN session_state;
