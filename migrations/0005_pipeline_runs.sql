CREATE TABLE pipeline_runs (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    run_number INTEGER NOT NULL,
    branch TEXT NOT NULL,
    commit_sha TEXT,
    trigger_type TEXT NOT NULL DEFAULT 'manual' CHECK (trigger_type IN ('manual', 'push', 'tag', 'schedule', 'webhook')),
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'running', 'success', 'failed', 'cancelled', 'skipped')),
    retry_of_run_id TEXT REFERENCES pipeline_runs(id) ON DELETE SET NULL,
    created_by TEXT NOT NULL REFERENCES users(id),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    started_at TEXT,
    finished_at TEXT,
    UNIQUE(project_id, run_number)
);

CREATE INDEX idx_pipeline_runs_project_created
    ON pipeline_runs(project_id, created_at DESC);
CREATE INDEX idx_pipeline_runs_status
    ON pipeline_runs(status);

CREATE TABLE pipeline_run_counters (
    project_id TEXT PRIMARY KEY NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    last_run_number INTEGER NOT NULL CHECK (last_run_number > 0)
);
