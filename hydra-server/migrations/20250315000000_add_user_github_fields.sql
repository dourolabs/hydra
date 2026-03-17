-- Bump user payload schema version for GitHub identity fields.
UPDATE metis.payload_schema_versions
SET current_version = 2
WHERE object_type = 'user';
