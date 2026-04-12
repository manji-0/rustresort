-- Migration 030: Persist quoted status relationships for quote notifications

ALTER TABLE statuses ADD COLUMN quote_of_uri TEXT;

CREATE INDEX IF NOT EXISTS idx_statuses_quote_of_uri
    ON statuses(quote_of_uri);
