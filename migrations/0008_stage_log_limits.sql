ALTER TABLE stage_runs
    ADD COLUMN logs_truncated INTEGER NOT NULL DEFAULT 0 CHECK (logs_truncated IN (0, 1));
