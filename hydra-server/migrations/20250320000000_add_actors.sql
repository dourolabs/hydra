-- Add actor payload support.
INSERT INTO hydra.payload_schema_versions (object_type, current_version)
VALUES ('actor', 1)
ON CONFLICT (object_type) DO NOTHING;

CREATE TABLE IF NOT EXISTS hydra.actors (
    id TEXT PRIMARY KEY,
    schema_version INTEGER NOT NULL DEFAULT hydra.current_schema_version('actor'),
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (schema_version > 0)
);

DROP TRIGGER IF EXISTS set_timestamp_actors ON hydra.actors;
CREATE TRIGGER set_timestamp_actors
BEFORE UPDATE ON hydra.actors
FOR EACH ROW
EXECUTE FUNCTION hydra.touch_updated_at();
