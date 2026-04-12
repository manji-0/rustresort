-- Track inbound ActivityPub notification deduplication and outbound follow acceptance.

ALTER TABLE notifications
    ADD COLUMN activity_uri TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_notifications_activity_uri
    ON notifications(activity_uri)
    WHERE activity_uri IS NOT NULL;

ALTER TABLE follows
    ADD COLUMN accepted_at TEXT;

CREATE INDEX IF NOT EXISTS idx_follows_accepted_at
    ON follows(accepted_at DESC)
    WHERE accepted_at IS NOT NULL;
