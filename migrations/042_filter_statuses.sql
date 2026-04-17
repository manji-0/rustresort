CREATE TABLE IF NOT EXISTS filter_statuses (
    id TEXT PRIMARY KEY,
    filter_id TEXT NOT NULL,
    status_id TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (filter_id) REFERENCES filters(id) ON DELETE CASCADE,
    UNIQUE(filter_id, status_id)
);

CREATE INDEX IF NOT EXISTS idx_filter_statuses_filter_id ON filter_statuses(filter_id);
