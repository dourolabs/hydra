-- Bump actor payload schema version to support auth token salt storage.
INSERT INTO hydra.payload_schema_versions (object_type, current_version)
VALUES ('actor', 2)
ON CONFLICT (object_type) DO UPDATE
SET current_version = EXCLUDED.current_version;
