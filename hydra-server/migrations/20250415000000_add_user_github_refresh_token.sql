-- Bump user payload schema version for GitHub refresh token storage.
UPDATE hydra.payload_schema_versions
SET current_version = 3
WHERE object_type = 'user';
