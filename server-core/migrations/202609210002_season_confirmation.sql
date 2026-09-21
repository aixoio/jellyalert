ALTER TABLE notifications ADD COLUMN awaiting_confirmation INTEGER NOT NULL DEFAULT 0 CHECK (awaiting_confirmation IN (0, 1));
