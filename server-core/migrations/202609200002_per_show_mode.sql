ALTER TABLE shows ADD COLUMN mode TEXT NOT NULL DEFAULT 'episode'
    CHECK (mode IN ('episode', 'season'));
-- Preserve the previously selected policy for every existing series.
UPDATE shows SET mode = COALESCE((SELECT mode FROM settings WHERE id = 1), 'episode');
ALTER TABLE settings DROP COLUMN mode;
