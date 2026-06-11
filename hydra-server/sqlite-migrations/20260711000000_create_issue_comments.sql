-- Per-issue append-only comments stream. See [[i-bckluuha]] for the
-- feature spec and [[i-aftcbtge]] (PR-1) for the wire types + memory
-- store impl. Sister Postgres migration ships in PR-3.
--
-- Append-only: no UPDATE / DELETE paths in this PR. `(issue_id,
-- sequence)` is the storage identity; `sequence` starts at 1 per
-- issue and increments monotonically within the issue.
--
-- `actor` stores the JSON-encoded `ActorRef` as TEXT (consistent with
-- the SQLite convention used for other ActorRef columns in this
-- store — see `issues_v2.actor`).
--
-- `created_at` follows the existing SQLite convention used by
-- `issues_v2.created_at` (ISO-8601 with millisecond precision and
-- `+00:00` suffix).

CREATE TABLE IF NOT EXISTS issue_comments (
    issue_id    TEXT    NOT NULL,
    sequence    INTEGER NOT NULL,
    body        TEXT    NOT NULL,
    actor       TEXT    NOT NULL,
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    PRIMARY KEY (issue_id, sequence)
);

-- Covering index for the DESC-by-sequence list endpoint.
CREATE INDEX IF NOT EXISTS issue_comments_issue_seq_desc_idx
    ON issue_comments (issue_id, sequence DESC);
