-- Built-in local authentication and passkey storage

CREATE TABLE IF NOT EXISTS passkeys (
    id TEXT PRIMARY KEY,
    credential_id TEXT NOT NULL UNIQUE,
    name TEXT,
    passkey_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    last_used_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_passkeys_created_at ON passkeys(created_at DESC);
