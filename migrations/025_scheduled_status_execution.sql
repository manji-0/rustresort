ALTER TABLE scheduled_statuses
    ADD COLUMN language TEXT;

ALTER TABLE scheduled_statuses
    ADD COLUMN error TEXT;

ALTER TABLE scheduled_statuses
    ADD COLUMN published_at TEXT;

CREATE INDEX IF NOT EXISTS idx_scheduled_statuses_pending
    ON scheduled_statuses(published_at, error, scheduled_at);
