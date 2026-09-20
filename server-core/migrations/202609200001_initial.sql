CREATE TABLE settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    mode TEXT NOT NULL DEFAULT 'episode' CHECK (mode IN ('episode', 'season')),
    tracking_since INTEGER NOT NULL,
    webhook_retry_at INTEGER NOT NULL DEFAULT 0,
    webhook_disabled INTEGER NOT NULL DEFAULT 0 CHECK (webhook_disabled IN (0, 1))
);
CREATE TABLE shows (
    id INTEGER PRIMARY KEY,
    title TEXT NOT NULL,
    excluded INTEGER NOT NULL DEFAULT 0 CHECK (excluded IN (0, 1)),
    active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0, 1))
);
CREATE TABLE notifications (
    key TEXT PRIMARY KEY,
    series_id INTEGER NOT NULL REFERENCES shows(id),
    mode TEXT NOT NULL CHECK (mode IN ('episode', 'season')),
    season INTEGER NOT NULL,
    episode_id INTEGER,
    due_at INTEGER NOT NULL,
    content TEXT NOT NULL,
    retry_at INTEGER NOT NULL DEFAULT 0,
    state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'sending', 'sent', 'uncertain', 'failed', 'covered')),
    attempted_at INTEGER,
    sent_at INTEGER
);
CREATE INDEX notifications_due ON notifications(state, due_at);
CREATE TABLE covered_episodes (
    episode_id INTEGER PRIMARY KEY,
    notification_key TEXT NOT NULL REFERENCES notifications(key)
);
