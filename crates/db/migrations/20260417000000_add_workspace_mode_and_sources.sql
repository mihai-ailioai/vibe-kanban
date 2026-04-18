ALTER TABLE workspaces
ADD COLUMN workspace_mode TEXT NOT NULL DEFAULT 'git_worktree'
CHECK (workspace_mode IN ('git_worktree', 'in_place_git', 'in_place_directory'));

CREATE TABLE workspace_sources (
    id            BLOB PRIMARY KEY,
    workspace_id  BLOB NOT NULL,
    source_type   TEXT NOT NULL
                  CHECK (source_type IN ('git_repo', 'directory')),
    repo_id       BLOB,
    path          TEXT,
    display_name  TEXT,
    target_branch TEXT,
    position      INTEGER NOT NULL CHECK (position >= 0),
    created_at    TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    updated_at    TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
    FOREIGN KEY (workspace_id) REFERENCES workspaces(id) ON DELETE CASCADE,
    FOREIGN KEY (repo_id) REFERENCES repos(id) ON DELETE CASCADE,
    CHECK (
        (
            source_type = 'git_repo'
            AND repo_id IS NOT NULL
            AND path IS NULL
            AND target_branch IS NOT NULL
        )
        OR
        (
            source_type = 'directory'
            AND repo_id IS NULL
            AND path IS NOT NULL
            AND target_branch IS NULL
        )
    )
);

CREATE INDEX idx_workspace_sources_workspace_id
ON workspace_sources (workspace_id);

CREATE INDEX idx_workspace_sources_repo_id
ON workspace_sources (repo_id)
WHERE repo_id IS NOT NULL;

CREATE UNIQUE INDEX idx_workspace_sources_workspace_position
ON workspace_sources (workspace_id, position);

CREATE UNIQUE INDEX idx_workspace_sources_git_uniqueness
ON workspace_sources (workspace_id, repo_id)
WHERE source_type = 'git_repo';
