-- Migration 009: idempotency storage for status creation

CREATE TABLE IF NOT EXISTS idempotency_keys (
    endpoint TEXT NOT NULL,
    key TEXT NOT NULL,
    response_json TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (endpoint, key)
);

CREATE INDEX IF NOT EXISTS idx_idempotency_keys_created_at
    ON idempotency_keys(created_at);

-- Automatically prune idempotency keys older than 30 days on insert.
CREATE TRIGGER IF NOT EXISTS trg_idempotency_keys_prune_old
AFTER INSERT ON idempotency_keys
BEGIN
    DELETE FROM idempotency_keys
    WHERE created_at < datetime('now', '-30 days');
END;
