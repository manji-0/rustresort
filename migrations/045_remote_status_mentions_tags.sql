CREATE TABLE IF NOT EXISTS remote_status_mentions (
    id TEXT PRIMARY KEY,
    status_id TEXT NOT NULL,
    actor_uri TEXT NOT NULL,
    username TEXT NOT NULL,
    acct TEXT NOT NULL,
    url TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (status_id) REFERENCES statuses(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_remote_status_mentions_status_id
    ON remote_status_mentions(status_id);

CREATE TABLE IF NOT EXISTS remote_status_tags (
    id TEXT PRIMARY KEY,
    status_id TEXT NOT NULL,
    name TEXT NOT NULL,
    url TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (status_id) REFERENCES statuses(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_remote_status_tags_status_id
    ON remote_status_tags(status_id);
