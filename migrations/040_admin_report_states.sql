CREATE TABLE IF NOT EXISTS admin_report_states (
    report_id TEXT PRIMARY KEY,
    category TEXT NOT NULL DEFAULT 'other',
    comment TEXT NOT NULL DEFAULT '',
    forwarded INTEGER NOT NULL DEFAULT 0,
    rule_ids_json TEXT,
    assigned_account_id TEXT,
    action_taken INTEGER NOT NULL DEFAULT 0,
    action_taken_at TEXT,
    action_taken_by_account_id TEXT,
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (report_id) REFERENCES notifications(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_admin_report_states_action_taken
    ON admin_report_states(action_taken, updated_at DESC);
