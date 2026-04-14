-- Add PKCE support and refresh-token fields for OAuth.

ALTER TABLE oauth_authorization_codes
    ADD COLUMN code_challenge TEXT;

ALTER TABLE oauth_authorization_codes
    ADD COLUMN code_challenge_method TEXT;

ALTER TABLE oauth_tokens
    ADD COLUMN refresh_token TEXT;

ALTER TABLE oauth_tokens
    ADD COLUMN refresh_expires_at TEXT;

CREATE INDEX IF NOT EXISTS idx_oauth_tokens_refresh_token
    ON oauth_tokens(refresh_token);
