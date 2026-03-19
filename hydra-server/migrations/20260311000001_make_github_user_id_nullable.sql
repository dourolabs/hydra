-- Make github_user_id nullable so users without GitHub identity can have NULL
ALTER TABLE metis.users_v2 ALTER COLUMN github_user_id DROP NOT NULL;

-- Convert existing sentinel value 0 to NULL
UPDATE metis.users_v2 SET github_user_id = NULL WHERE github_user_id = 0;
