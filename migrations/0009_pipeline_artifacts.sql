CREATE TABLE pipeline_artifacts (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES pipeline_runs(id) ON DELETE CASCADE,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    file_name TEXT NOT NULL,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    checksum_sha256 TEXT NOT NULL CHECK (length(checksum_sha256) = 64),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(run_id, name)
);

CREATE INDEX idx_pipeline_artifacts_project_created
    ON pipeline_artifacts(project_id, created_at DESC);
