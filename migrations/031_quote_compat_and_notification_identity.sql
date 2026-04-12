-- Migration 031: preserve scheduled quotes and allow per-identity notification fan-out

ALTER TABLE scheduled_statuses
    ADD COLUMN quoted_status_id TEXT;

DROP INDEX IF EXISTS idx_notifications_activity_uri;

CREATE UNIQUE INDEX IF NOT EXISTS idx_notifications_activity_identity
    ON notifications(
        activity_uri,
        notification_type,
        origin_account_address,
        COALESCE(status_uri, '')
    )
    WHERE activity_uri IS NOT NULL;
