ALTER TABLE notifications ADD COLUMN plan_snapshot TEXT NOT NULL DEFAULT '';

-- The application captures the old global preference before the original,
-- checksum-verified migrations remove settings.mode. The snapshot survives a
-- crash during migration; existing installations without it keep their default.
CREATE TABLE IF NOT EXISTS legacy_default_mode (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    mode TEXT NOT NULL CHECK (mode IN ('episode', 'season'))
);
UPDATE settings SET default_mode = (SELECT mode FROM legacy_default_mode WHERE id = 1)
WHERE id = 1 AND EXISTS (SELECT 1 FROM legacy_default_mode WHERE id = 1);
DROP TABLE legacy_default_mode;
