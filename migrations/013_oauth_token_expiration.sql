-- Add expiration timestamp to OAuth access tokens.
-- Existing tokens are marked as expired at migration time.
ALTER TABLE oauth_tokens
    ADD COLUMN expires_at TEXT;

UPDATE oauth_tokens
SET expires_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
WHERE expires_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_oauth_tokens_expires_at ON oauth_tokens(expires_at);
