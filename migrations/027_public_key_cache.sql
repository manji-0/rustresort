CREATE TABLE IF NOT EXISTS public_key_cache (
    key_id TEXT PRIMARY KEY,
    pem TEXT NOT NULL,
    expires_at TIMESTAMP NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_public_key_cache_expires_at
    ON public_key_cache(expires_at);
