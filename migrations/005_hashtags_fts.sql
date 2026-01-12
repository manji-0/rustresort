-- Migration 005: Hashtags and Full-Text Search
-- Adds tables for hashtag tracking and FTS5 virtual table for content search

-- ============================================================================
-- Hashtags
-- ============================================================================

-- Hashtags table
CREATE TABLE IF NOT EXISTS hashtags (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Status-hashtag relationship table
CREATE TABLE IF NOT EXISTS status_hashtags (
    id TEXT PRIMARY KEY,
    status_id TEXT NOT NULL,
    hashtag_id TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (status_id) REFERENCES statuses(id) ON DELETE CASCADE,
    FOREIGN KEY (hashtag_id) REFERENCES hashtags(id) ON DELETE CASCADE,
    UNIQUE(status_id, hashtag_id)
);

-- ============================================================================
-- Full-Text Search
-- ============================================================================

-- Create FTS5 virtual table for status content search
CREATE VIRTUAL TABLE IF NOT EXISTS statuses_fts USING fts5(
    status_id UNINDEXED,
    content,
    content='statuses',
    content_rowid='rowid'
);

-- Populate FTS index with existing statuses
INSERT INTO statuses_fts(status_id, content)
SELECT id, content FROM statuses;

-- Trigger to keep FTS index in sync when statuses are inserted
CREATE TRIGGER IF NOT EXISTS statuses_ai AFTER INSERT ON statuses BEGIN
    INSERT INTO statuses_fts(rowid, status_id, content) VALUES (new.rowid, new.id, new.content);
END;

-- Trigger to keep FTS index in sync when statuses are updated
-- FTS5 external content requires delete + insert for updates
CREATE TRIGGER IF NOT EXISTS statuses_au AFTER UPDATE ON statuses BEGIN
    INSERT INTO statuses_fts(statuses_fts, rowid, status_id, content) VALUES('delete', old.rowid, old.id, old.content);
    INSERT INTO statuses_fts(rowid, status_id, content) VALUES (new.rowid, new.id, new.content);
END;

-- Trigger to keep FTS index in sync when statuses are deleted
-- FTS5 external content requires special delete syntax
CREATE TRIGGER IF NOT EXISTS statuses_ad AFTER DELETE ON statuses BEGIN
    INSERT INTO statuses_fts(statuses_fts, rowid, status_id, content) VALUES('delete', old.rowid, old.id, old.content);
END;

-- ============================================================================
-- Indexes
-- ============================================================================

-- Create index on hashtags for faster lookups
CREATE INDEX IF NOT EXISTS idx_hashtags_name ON hashtags(name COLLATE NOCASE);

-- Create index on status_hashtags for faster joins
CREATE INDEX IF NOT EXISTS idx_status_hashtags_hashtag_id ON status_hashtags(hashtag_id);
CREATE INDEX IF NOT EXISTS idx_status_hashtags_status_id ON status_hashtags(status_id);

-- ============================================================================
-- Views
-- ============================================================================

-- Create view for hashtag statistics
CREATE VIEW IF NOT EXISTS hashtag_stats AS
SELECT 
    h.id,
    h.name,
    COUNT(DISTINCT sh.status_id) as usage_count,
    MAX(s.created_at) as last_used_at
FROM hashtags h
LEFT JOIN status_hashtags sh ON h.id = sh.hashtag_id
LEFT JOIN statuses s ON sh.status_id = s.id
GROUP BY h.id, h.name;
