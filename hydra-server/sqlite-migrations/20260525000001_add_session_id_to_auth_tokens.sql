-- Phase 3a of the actor-system overhaul (designs/actor-system-overhaul.md §7.2):
-- track which session minted each auth token so Phase 3b can revoke by
-- session_id in O(1).

ALTER TABLE auth_tokens ADD COLUMN session_id TEXT;

CREATE INDEX IF NOT EXISTS auth_tokens_session_id_idx ON auth_tokens (session_id);
