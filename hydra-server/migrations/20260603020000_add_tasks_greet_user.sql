-- WS-only worker lifecycle cutover: SessionMode::Interactive gains an
-- optional `greet_user: bool` field that decides whether the agent
-- produces a greeting turn before any user message arrives. The Rust
-- type defaults the field to `false` via `#[serde(default)]`, so
-- existing JSONB rows in `metis.tasks_v2.mode` continue to deserialize
-- correctly without a JSON-side backfill.
--
-- The explicit column is added for query/analytics access against the
-- new flag (no application read path uses it as of this migration).
-- Append-only per the migrations policy.

ALTER TABLE metis.tasks_v2 ADD COLUMN greet_user BOOLEAN NOT NULL DEFAULT FALSE;
