CREATE TABLE sessions (
    token_hash TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX idx_sessions_user_id ON sessions(user_id);
CREATE INDEX idx_sessions_expires_at ON sessions(expires_at);

CREATE TABLE setup_state (
    singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
    claimed INTEGER NOT NULL DEFAULT 0 CHECK (claimed IN (0, 1))
);

INSERT INTO setup_state (singleton, claimed) VALUES (1, 0);
