-- Migration 004: Polls, Scheduled Statuses, and Conversations
-- Adds tables for poll voting, scheduled posting, and direct message conversations

-- ============================================================================
-- Polls
-- ============================================================================

-- Polls table
CREATE TABLE IF NOT EXISTS polls (
    id TEXT PRIMARY KEY,
    status_id TEXT,
    expires_at TEXT NOT NULL,
    expired INTEGER NOT NULL DEFAULT 0,
    multiple INTEGER NOT NULL DEFAULT 0,
    votes_count INTEGER NOT NULL DEFAULT 0,
    voters_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (status_id) REFERENCES statuses(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_polls_status_id ON polls(status_id);
CREATE INDEX IF NOT EXISTS idx_polls_expires_at ON polls(expires_at);

-- Poll options table
CREATE TABLE IF NOT EXISTS poll_options (
    id TEXT PRIMARY KEY,
    poll_id TEXT NOT NULL,
    title TEXT NOT NULL,
    votes_count INTEGER NOT NULL DEFAULT 0,
    option_index INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (poll_id) REFERENCES polls(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_poll_options_poll_id ON poll_options(poll_id);

-- Poll votes table
CREATE TABLE IF NOT EXISTS poll_votes (
    id TEXT PRIMARY KEY,
    poll_id TEXT NOT NULL,
    option_id TEXT NOT NULL,
    voter_address TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (poll_id) REFERENCES polls(id) ON DELETE CASCADE,
    FOREIGN KEY (option_id) REFERENCES poll_options(id) ON DELETE CASCADE,
    UNIQUE(poll_id, voter_address, option_id)
);

CREATE INDEX IF NOT EXISTS idx_poll_votes_poll_id ON poll_votes(poll_id);
CREATE INDEX IF NOT EXISTS idx_poll_votes_voter ON poll_votes(voter_address);

-- ============================================================================
-- Scheduled Statuses
-- ============================================================================

-- Scheduled statuses table
CREATE TABLE IF NOT EXISTS scheduled_statuses (
    id TEXT PRIMARY KEY,
    scheduled_at TEXT NOT NULL,
    status_text TEXT NOT NULL,
    visibility TEXT NOT NULL DEFAULT 'public',
    content_warning TEXT,
    in_reply_to_id TEXT,
    media_ids TEXT, -- JSON array of media IDs
    poll_options TEXT, -- JSON array of poll options
    poll_expires_in INTEGER,
    poll_multiple INTEGER DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_scheduled_statuses_scheduled_at ON scheduled_statuses(scheduled_at);

-- ============================================================================
-- Conversations (Direct Messages)
-- ============================================================================

-- Conversations table
CREATE TABLE IF NOT EXISTS conversations (
    id TEXT PRIMARY KEY,
    last_status_id TEXT,
    unread INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_conversations_updated_at ON conversations(updated_at);

-- Conversation participants table
CREATE TABLE IF NOT EXISTS conversation_participants (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    account_address TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE CASCADE,
    UNIQUE(conversation_id, account_address)
);

CREATE INDEX IF NOT EXISTS idx_conversation_participants_conversation_id ON conversation_participants(conversation_id);
CREATE INDEX IF NOT EXISTS idx_conversation_participants_account ON conversation_participants(account_address);

-- Conversation statuses (link statuses to conversations)
CREATE TABLE IF NOT EXISTS conversation_statuses (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    status_id TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE CASCADE,
    FOREIGN KEY (status_id) REFERENCES statuses(id) ON DELETE CASCADE,
    UNIQUE(conversation_id, status_id)
);

CREATE INDEX IF NOT EXISTS idx_conversation_statuses_conversation_id ON conversation_statuses(conversation_id);
CREATE INDEX IF NOT EXISTS idx_conversation_statuses_status_id ON conversation_statuses(status_id);
