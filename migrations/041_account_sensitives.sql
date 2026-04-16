CREATE TABLE IF NOT EXISTS account_sensitives (
    id TEXT PRIMARY KEY,
    target_address TEXT NOT NULL UNIQUE,
    actor_uri TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_account_sensitives_target_address
    ON account_sensitives(target_address);
