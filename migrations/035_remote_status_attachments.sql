CREATE TABLE IF NOT EXISTS remote_status_attachments (
    id TEXT PRIMARY KEY,
    status_id TEXT NOT NULL,
    remote_url TEXT NOT NULL,
    preview_url TEXT,
    content_type TEXT NOT NULL,
    description TEXT,
    blurhash TEXT,
    width INTEGER,
    height INTEGER,
    created_at TEXT NOT NULL,
    FOREIGN KEY (status_id) REFERENCES statuses(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_remote_status_attachments_status_id
    ON remote_status_attachments(status_id);
