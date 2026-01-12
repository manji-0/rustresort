-- Add OAuth apps and tokens tables

CREATE TABLE IF NOT EXISTS oauth_apps (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    website TEXT,
    redirect_uri TEXT NOT NULL,
    client_id TEXT NOT NULL UNIQUE,
    client_secret TEXT NOT NULL,
    scopes TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS oauth_tokens (
    id TEXT PRIMARY KEY,
    app_id TEXT NOT NULL,
    access_token TEXT NOT NULL UNIQUE,
    scopes TEXT NOT NULL,
    created_at TEXT NOT NULL,
    revoked INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (app_id) REFERENCES oauth_apps(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_oauth_tokens_access_token ON oauth_tokens(access_token);
CREATE INDEX IF NOT EXISTS idx_oauth_tokens_app_id ON oauth_tokens(app_id);
