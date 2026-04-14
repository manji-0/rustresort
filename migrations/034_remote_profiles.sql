CREATE TABLE IF NOT EXISTS remote_profiles (
    address TEXT PRIMARY KEY,
    uri TEXT NOT NULL,
    display_name TEXT,
    note TEXT,
    avatar_url TEXT,
    header_url TEXT,
    public_key_pem TEXT NOT NULL,
    inbox_uri TEXT NOT NULL,
    outbox_uri TEXT,
    followers_count INTEGER,
    following_count INTEGER,
    fetched_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_remote_profiles_uri
    ON remote_profiles(uri);
