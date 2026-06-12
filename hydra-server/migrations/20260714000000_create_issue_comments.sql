-- Sister to SQLite `20260711000000_create_issue_comments.sql`. Creates
-- `metis.issue_comments` as the canonical store for the per-issue
-- append-only comments stream. See [[i-bckluuha]] for the feature
-- spec and [[i-aftcbtge]] (PR-1) for the wire types + MemoryStore
-- impl; [[i-klqgmpce]] (PR-2) shipped the SQLite half.
--
-- Append-only: no UPDATE / DELETE paths. `(issue_id, sequence)` is
-- the storage identity; `sequence` starts at 1 per issue and
-- increments monotonically within the issue.
--
-- Postgres differences vs. the SQLite sister:
--   * `metis.` schema prefix.
--   * `sequence` is BIGINT (matches the existing `metis.statuses`
--     convention; SQLite collapses INTEGER to i64 anyway).
--   * `actor` is JSONB (not TEXT) — matches the existing
--     `metis.projects.actor` JSONB convention so PG-side reads can
--     index / introspect with native JSON ops if a future PR needs to.
--   * `created_at` is `TIMESTAMPTZ` with `DEFAULT NOW()`, mirroring
--     `metis.issues_v2.created_at`.

CREATE TABLE IF NOT EXISTS metis.issue_comments (
    issue_id    TEXT        NOT NULL,
    sequence    BIGINT      NOT NULL,
    body        TEXT        NOT NULL,
    actor       JSONB       NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (issue_id, sequence)
);

-- Covering index for the DESC-by-sequence list endpoint.
CREATE INDEX IF NOT EXISTS issue_comments_issue_seq_desc_idx
    ON metis.issue_comments (issue_id, sequence DESC);
