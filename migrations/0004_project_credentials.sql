ALTER TABLE projects ADD COLUMN git_auth_id TEXT REFERENCES credentials(id) ON DELETE SET NULL;

CREATE INDEX idx_projects_git_auth_id ON projects(git_auth_id);
