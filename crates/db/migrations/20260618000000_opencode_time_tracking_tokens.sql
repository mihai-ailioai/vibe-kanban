CREATE TABLE opencode_time_tracking_tokens (
  id BLOB PRIMARY KEY,
  token_hash TEXT NOT NULL UNIQUE,
  scope TEXT NOT NULL CHECK (scope = 'time_tracking:write'),
  label TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
  last_used_at TEXT,
  revoked_at TEXT
);

CREATE INDEX idx_opencode_time_tracking_tokens_hash
  ON opencode_time_tracking_tokens(token_hash)
  WHERE revoked_at IS NULL;
