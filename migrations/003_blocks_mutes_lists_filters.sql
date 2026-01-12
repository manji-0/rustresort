-- Migration 003: Blocks, Mutes, Lists, and Filters
-- Adds tables for account moderation, user-defined lists, and content filtering

-- Account blocks table
CREATE TABLE IF NOT EXISTS account_blocks (
    id TEXT PRIMARY KEY,
    target_address TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_account_blocks_target_address ON account_blocks(target_address);

-- Account mutes table
CREATE TABLE IF NOT EXISTS account_mutes (
    id TEXT PRIMARY KEY,
    target_address TEXT NOT NULL UNIQUE,
    notifications INTEGER NOT NULL DEFAULT 1, -- 1 = mute notifications, 0 = don't mute notifications
    duration INTEGER, -- NULL = indefinite, otherwise seconds until unmute
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_account_mutes_target_address ON account_mutes(target_address);

-- Follow requests table (for accounts that require approval)
CREATE TABLE IF NOT EXISTS follow_requests (
    id TEXT PRIMARY KEY,
    requester_address TEXT NOT NULL UNIQUE,
    inbox_uri TEXT NOT NULL,
    uri TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_follow_requests_requester_address ON follow_requests(requester_address);

-- Lists table
CREATE TABLE IF NOT EXISTS lists (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    replies_policy TEXT NOT NULL DEFAULT 'list', -- 'followed', 'list', 'none'
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- List accounts table (many-to-many relationship)
CREATE TABLE IF NOT EXISTS list_accounts (
    id TEXT PRIMARY KEY,
    list_id TEXT NOT NULL,
    account_address TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (list_id) REFERENCES lists(id) ON DELETE CASCADE,
    UNIQUE(list_id, account_address)
);

CREATE INDEX IF NOT EXISTS idx_list_accounts_list_id ON list_accounts(list_id);
CREATE INDEX IF NOT EXISTS idx_list_accounts_account_address ON list_accounts(account_address);

-- Filters table (v1 API)
CREATE TABLE IF NOT EXISTS filters (
    id TEXT PRIMARY KEY,
    phrase TEXT NOT NULL,
    context TEXT NOT NULL, -- JSON array: ["home", "notifications", "public", "thread"]
    expires_at TEXT, -- NULL = never expires
    irreversible INTEGER NOT NULL DEFAULT 0,
    whole_word INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Filter keywords table (v2 API)
CREATE TABLE IF NOT EXISTS filter_keywords (
    id TEXT PRIMARY KEY,
    filter_id TEXT NOT NULL,
    keyword TEXT NOT NULL,
    whole_word INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (filter_id) REFERENCES filters(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_filter_keywords_filter_id ON filter_keywords(filter_id);
