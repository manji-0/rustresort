-- Track OAuth token origin grant to distinguish app-only tokens
-- from user-authorized tokens.
-- Legacy rows predate grant tracking and can include mixed grant types.
-- Treat unknown legacy rows as non-user grants and revoke them to avoid
-- reclassifying app-only tokens as user-authorized sessions.
ALTER TABLE oauth_tokens
    ADD COLUMN grant_type TEXT NOT NULL DEFAULT 'client_credentials';

UPDATE oauth_tokens
SET revoked = 1
WHERE grant_type = 'client_credentials';
