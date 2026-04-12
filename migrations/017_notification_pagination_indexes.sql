-- Improve notification pagination and unread filtering.
CREATE INDEX IF NOT EXISTS idx_notifications_created_at_id_desc
    ON notifications(created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_notifications_read_created_at_id_desc
    ON notifications(read, created_at DESC, id DESC);
