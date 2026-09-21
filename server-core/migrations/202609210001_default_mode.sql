ALTER TABLE settings ADD COLUMN default_mode TEXT NOT NULL DEFAULT 'episode'
    CHECK (default_mode IN ('episode', 'season'));
ALTER TABLE shows ADD COLUMN mode_overridden INTEGER NOT NULL DEFAULT 0
    CHECK (mode_overridden IN (0, 1));
-- Older versions did not track intent: preserve every existing preference.
UPDATE shows SET mode_overridden = 1;
