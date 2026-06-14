-- Final cutover of the progress/feedback → comments migration. The
-- `seed_progress_as_comments` Rust migration (version 20260720000000)
-- ran immediately before this SQL step and seeded one
-- `metis.issue_comments` row per issue whose `progress` was ever
-- populated. Feedback is dropped outright (no comment seeded) per the
-- parent spec.

ALTER TABLE metis.issues_v2 DROP COLUMN IF EXISTS progress;
ALTER TABLE metis.issues_v2 DROP COLUMN IF EXISTS feedback;
