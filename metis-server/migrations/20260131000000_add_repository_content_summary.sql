-- Bump repository payload schema version for optional content summaries.
UPDATE metis.payload_schema_versions
SET current_version = 2
WHERE object_type = 'repository';

-- Ensure repository payloads gain a content_summary key when migrating and retain
-- prior actor migration logic for auth_token_salt.
CREATE OR REPLACE FUNCTION metis.migrate_payload(
    object_type TEXT,
    from_version INTEGER,
    to_version INTEGER,
    payload JSONB
) RETURNS JSONB AS $$
DECLARE
    updated JSONB := payload;
    salt TEXT;
BEGIN
    IF object_type = 'actor' AND to_version >= 3 AND from_version < 3 THEN
        IF NOT (updated ? 'auth_token_salt') OR (updated->>'auth_token_salt') IS NULL
            OR (updated->>'auth_token_salt') = '' THEN
            salt := md5(random()::text || clock_timestamp()::text);
            updated := jsonb_set(updated, '{auth_token_salt}', to_jsonb(salt), true);
        END IF;
    END IF;

    IF object_type = 'repository' AND to_version >= 2 AND from_version < 2 THEN
        IF NOT (updated ? 'content_summary') THEN
            updated := jsonb_set(updated, '{content_summary}', 'null'::jsonb, true);
        END IF;
    END IF;

    RETURN updated;
END;
$$ LANGUAGE plpgsql STABLE;
