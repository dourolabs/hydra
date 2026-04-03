-- Add auth_tokens table for multi-token storage per actor.
-- Copies existing token hashes from actors_v2 into the new table.

CREATE TABLE IF NOT EXISTS auth_tokens (
    actor_name TEXT NOT NULL,
    token_hash TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    PRIMARY KEY (actor_name, token_hash)
);

CREATE INDEX IF NOT EXISTS auth_tokens_actor_name_idx ON auth_tokens (actor_name);

-- Migrate existing token hashes from the latest version of each actor.
INSERT OR IGNORE INTO auth_tokens (actor_name, token_hash, created_at)
SELECT a.id, a.auth_token_hash, a.created_at
FROM actors_v2 a
INNER JOIN (
    SELECT id, MAX(version_number) AS max_version
    FROM actors_v2
    GROUP BY id
) latest ON a.id = latest.id AND a.version_number = latest.max_version
WHERE a.auth_token_hash != '';
