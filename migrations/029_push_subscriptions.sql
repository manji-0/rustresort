CREATE TABLE IF NOT EXISTS push_subscriptions (
    id TEXT PRIMARY KEY,
    endpoint TEXT NOT NULL UNIQUE,
    p256dh TEXT NOT NULL,
    auth TEXT NOT NULL,
    alerts_json TEXT NOT NULL,
    policy TEXT NOT NULL DEFAULT 'all',
    created_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP NOT NULL
);
