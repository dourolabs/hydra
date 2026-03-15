-- Add internal flag to user_secrets to distinguish system-internal secrets
-- (e.g. GITHUB_TOKEN, GITHUB_REFRESH_TOKEN) from user-managed secrets.
ALTER TABLE user_secrets ADD COLUMN internal BOOLEAN NOT NULL DEFAULT FALSE;

-- Mark existing system-internal secrets as internal.
UPDATE user_secrets SET internal = TRUE WHERE secret_name IN ('GITHUB_TOKEN', 'GITHUB_REFRESH_TOKEN');
