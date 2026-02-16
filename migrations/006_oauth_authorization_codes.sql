-- Add OAuth authorization codes for authorization_code flow

CREATE TABLE IF NOT EXISTS oauth_authorization_codes (
    id TEXT PRIMARY KEY,
    app_id TEXT NOT NULL,
    code TEXT NOT NULL UNIQUE,
    redirect_uri TEXT NOT NULL,
    scopes TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    FOREIGN KEY (app_id) REFERENCES oauth_apps(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_oauth_authorization_codes_code
    ON oauth_authorization_codes(code);
CREATE INDEX IF NOT EXISTS idx_oauth_authorization_codes_app_id
    ON oauth_authorization_codes(app_id);
