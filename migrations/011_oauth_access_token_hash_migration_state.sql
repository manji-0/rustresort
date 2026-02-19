-- Mark OAuth access-token hash-at-rest backfill as pending.
-- The actual backfill is performed by application code because SQLite
-- in this environment does not provide a built-in SHA-256 SQL function.
INSERT OR IGNORE INTO settings (key, value)
VALUES ('oauth_tokens_access_token_hash_migration', 'pending');
