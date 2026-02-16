-- Add NOCASE indexes for case-insensitive follow/follower address lookups.
-- These indexes support DELETE/lookup queries that use COLLATE NOCASE
-- without forcing full table scans.

CREATE INDEX IF NOT EXISTS idx_follows_target_address_nocase
    ON follows(target_address COLLATE NOCASE);

CREATE INDEX IF NOT EXISTS idx_followers_follower_address_nocase
    ON followers(follower_address COLLATE NOCASE);
