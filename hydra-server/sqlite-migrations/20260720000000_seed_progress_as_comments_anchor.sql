-- No-op anchor for the `seed_progress_as_comments` Rust migration (see
-- `src/store/migrations/seed_progress_as_comments.rs`). The Rust
-- migration runs in the interleaved migration plan immediately after
-- this SQL version, reading the `issues_v2.progress` column and
-- inserting one comment per issue whose progress was ever populated.
-- The column-drop migration at 20260721000000 runs after this Rust
-- step, so the read is safe.
SELECT 1;
