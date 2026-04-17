CREATE TABLE workspace_repo_claims (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL,
  repo_id TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
  FOREIGN KEY (workspace_id) REFERENCES workspaces(id) ON DELETE CASCADE,
  FOREIGN KEY (repo_id) REFERENCES repos(id) ON DELETE CASCADE,
  UNIQUE (repo_id)
);

CREATE INDEX idx_workspace_repo_claims_workspace_id
  ON workspace_repo_claims(workspace_id);
