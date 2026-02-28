-- Migration 012: Status context/history support and persisted pin/mute state

-- Edit history snapshots for statuses.
CREATE TABLE IF NOT EXISTS status_edits (
    id TEXT PRIMARY KEY,
    status_id TEXT NOT NULL,
    content TEXT NOT NULL,
    content_warning TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (status_id) REFERENCES statuses(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_status_edits_status_id_created_at
    ON status_edits(status_id, created_at DESC);

-- Profile-pinned statuses for the local account.
CREATE TABLE IF NOT EXISTS pinned_statuses (
    id TEXT PRIMARY KEY,
    status_id TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (status_id) REFERENCES statuses(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_pinned_statuses_created_at
    ON pinned_statuses(created_at DESC);

-- Conversation mute markers keyed by thread URI.
CREATE TABLE IF NOT EXISTS muted_conversations (
    id TEXT PRIMARY KEY,
    thread_uri TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_muted_conversations_created_at
    ON muted_conversations(created_at DESC);
