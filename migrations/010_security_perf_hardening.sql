-- Migration 010: security/performance hardening indexes

-- Speeds visibility-filtered local status scans used by ActivityPub outbox.
CREATE INDEX IF NOT EXISTS idx_statuses_local_visibility_created_at
    ON statuses(is_local, visibility, created_at DESC);

-- Speeds per-poll, per-voter vote checks.
CREATE INDEX IF NOT EXISTS idx_poll_votes_poll_voter
    ON poll_votes(poll_id, voter_address);
