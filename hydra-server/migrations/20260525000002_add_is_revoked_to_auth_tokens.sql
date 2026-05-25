-- Phase 3b of the actor-system overhaul (designs/actor-system-overhaul.md
-- §7.2, §7.4): per-token revocation flag. `sessions kill <id>` flips
-- `is_revoked` to TRUE for every token minted by the killed session so
-- the subsequent request from that session's container fails at the
-- auth layer rather than at the (now-deleted) `RunningJobValidationRestriction`
-- policy check.

ALTER TABLE metis.auth_tokens ADD COLUMN is_revoked BOOLEAN NOT NULL DEFAULT FALSE;
