-- Change primary key to (username, secret_name, internal) so both an internal
-- and external version of the same secret can coexist.
-- SQLite does not support ALTER TABLE to change a PK, so recreate the table.
CREATE TABLE IF NOT EXISTS user_secrets_new (
    username TEXT NOT NULL,
    secret_name TEXT NOT NULL,
    encrypted_value BLOB NOT NULL,
    internal BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    PRIMARY KEY (username, secret_name, internal)
);
INSERT INTO user_secrets_new SELECT * FROM user_secrets;
DROP TABLE user_secrets;
ALTER TABLE user_secrets_new RENAME TO user_secrets;
