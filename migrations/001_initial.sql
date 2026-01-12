-- RustResort Initial Schema
-- SQLite Migration

-- Account table (single user)
CREATE TABLE IF NOT EXISTS account (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    display_name TEXT,
    note TEXT,
    avatar_s3_key TEXT,
    header_s3_key TEXT,
    private_key_pem TEXT NOT NULL,
    public_key_pem TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Statuses table
CREATE TABLE IF NOT EXISTS statuses (
    id TEXT PRIMARY KEY,
    uri TEXT NOT NULL UNIQUE,
    content TEXT NOT NULL,
    content_warning TEXT,
    visibility TEXT NOT NULL DEFAULT 'public',
    language TEXT,
    account_address TEXT NOT NULL DEFAULT '',
    is_local INTEGER NOT NULL DEFAULT 0,
    in_reply_to_uri TEXT,
    boost_of_uri TEXT,
    persisted_reason TEXT NOT NULL DEFAULT 'own',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    fetched_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_statuses_uri ON statuses(uri);
CREATE INDEX IF NOT EXISTS idx_statuses_is_local ON statuses(is_local);
CREATE INDEX IF NOT EXISTS idx_statuses_created_at ON statuses(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_statuses_account_address ON statuses(account_address);

-- Media attachments table
CREATE TABLE IF NOT EXISTS media_attachments (
    id TEXT PRIMARY KEY,
    status_id TEXT,
    s3_key TEXT NOT NULL,
    thumbnail_s3_key TEXT,
    content_type TEXT NOT NULL,
    file_size INTEGER NOT NULL,
    description TEXT,
    blurhash TEXT,
    width INTEGER,
    height INTEGER,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (status_id) REFERENCES statuses(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_media_attachments_status_id ON media_attachments(status_id);

-- Follows table (users we follow)
CREATE TABLE IF NOT EXISTS follows (
    id TEXT PRIMARY KEY,
    target_address TEXT NOT NULL UNIQUE,
    uri TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_follows_target_address ON follows(target_address);

-- Followers table (users following us)
CREATE TABLE IF NOT EXISTS followers (
    id TEXT PRIMARY KEY,
    follower_address TEXT NOT NULL UNIQUE,
    inbox_uri TEXT NOT NULL,
    uri TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_followers_follower_address ON followers(follower_address);

-- Notifications table
CREATE TABLE IF NOT EXISTS notifications (
    id TEXT PRIMARY KEY,
    notification_type TEXT NOT NULL,
    origin_account_address TEXT NOT NULL,
    status_uri TEXT,
    read INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_notifications_created_at ON notifications(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_notifications_read ON notifications(read);

-- Favourites table
CREATE TABLE IF NOT EXISTS favourites (
    id TEXT PRIMARY KEY,
    status_id TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (status_id) REFERENCES statuses(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_favourites_status_id ON favourites(status_id);

-- Bookmarks table
CREATE TABLE IF NOT EXISTS bookmarks (
    id TEXT PRIMARY KEY,
    status_id TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (status_id) REFERENCES statuses(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_bookmarks_status_id ON bookmarks(status_id);

-- Reposts (boosts) table
CREATE TABLE IF NOT EXISTS reposts (
    id TEXT PRIMARY KEY,
    status_id TEXT NOT NULL,
    uri TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (status_id) REFERENCES statuses(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_reposts_status_id ON reposts(status_id);

-- Domain blocks table
CREATE TABLE IF NOT EXISTS domain_blocks (
    id TEXT PRIMARY KEY,
    domain TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Settings table (key-value)
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
