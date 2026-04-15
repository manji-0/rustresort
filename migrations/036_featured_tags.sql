-- Migration 036: Featured tags
-- Stores hashtags explicitly featured on the local profile.

CREATE TABLE IF NOT EXISTS featured_tags (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_featured_tags_name ON featured_tags(name COLLATE NOCASE);
