CREATE TABLE pipeline_stage_logs (
    id TEXT PRIMARY KEY NOT NULL,
    stage_run_id TEXT NOT NULL REFERENCES stage_runs(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL CHECK (sequence >= 0),
    stream TEXT NOT NULL CHECK (stream IN ('stdout', 'stderr')),
    data BLOB NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(stage_run_id, sequence)
);

CREATE INDEX idx_pipeline_stage_logs_stage_sequence
    ON pipeline_stage_logs(stage_run_id, sequence);
