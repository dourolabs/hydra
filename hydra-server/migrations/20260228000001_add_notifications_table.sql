-- Notifications table for event-driven notification system.
-- Non-versioned: the only mutation after creation is marking as read.
CREATE TABLE IF NOT EXISTS hydra.notifications (
    id              TEXT NOT NULL PRIMARY KEY,
    recipient       TEXT NOT NULL,
    source_actor    TEXT,
    object_kind     TEXT NOT NULL,
    object_id       TEXT NOT NULL,
    object_version  BIGINT NOT NULL,
    event_type      TEXT NOT NULL,
    summary         TEXT NOT NULL,
    source_issue_id TEXT,
    policy          TEXT NOT NULL DEFAULT 'walk_up',
    is_read         BOOLEAN NOT NULL DEFAULT FALSE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Optimized for querying unread notifications by recipient (most common query path).
CREATE INDEX IF NOT EXISTS idx_notifications_recipient_unread
    ON hydra.notifications (recipient, is_read, created_at DESC);

-- Optimized for querying all notifications by recipient.
CREATE INDEX IF NOT EXISTS idx_notifications_recipient_all
    ON hydra.notifications (recipient, created_at DESC);

-- Optimized for looking up notifications by the object that triggered them.
CREATE INDEX IF NOT EXISTS idx_notifications_object
    ON hydra.notifications (object_id, object_version);
