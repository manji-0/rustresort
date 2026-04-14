-- Track remote favourites and boosts separately from local user interactions.

CREATE TABLE IF NOT EXISTS remote_favourites (
    id TEXT PRIMARY KEY,
    status_id TEXT NOT NULL,
    actor_address TEXT NOT NULL,
    activity_uri TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (status_id) REFERENCES statuses(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_remote_favourites_actor_status
    ON remote_favourites(status_id, actor_address);
CREATE UNIQUE INDEX IF NOT EXISTS idx_remote_favourites_activity_uri
    ON remote_favourites(activity_uri)
    WHERE activity_uri IS NOT NULL;

CREATE TABLE IF NOT EXISTS remote_reposts (
    id TEXT PRIMARY KEY,
    status_id TEXT NOT NULL,
    actor_address TEXT NOT NULL,
    activity_uri TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (status_id) REFERENCES statuses(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_remote_reposts_actor_status
    ON remote_reposts(status_id, actor_address);
CREATE UNIQUE INDEX IF NOT EXISTS idx_remote_reposts_activity_uri
    ON remote_reposts(activity_uri)
    WHERE activity_uri IS NOT NULL;
