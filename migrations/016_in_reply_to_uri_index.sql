-- Index for efficient thread/reply lookups.
CREATE INDEX IF NOT EXISTS idx_statuses_in_reply_to_uri
  ON statuses(in_reply_to_uri)
  WHERE in_reply_to_uri IS NOT NULL;
