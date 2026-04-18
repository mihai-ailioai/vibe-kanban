import { describe, expect, it, vi } from 'vitest';
import type {
  RepoBranchStatus,
  Workspace,
  WorkspaceCapabilities,
} from 'shared/types';
import { loadDeleteWorkspaceDialogState } from './deleteWorkspaceDialogState';

function makeWorkspace(overrides: Partial<Workspace> = {}): Workspace {
  return {
    id: 'workspace-1',
    task_id: null,
    container_ref: null,
    branch: 'feature/test-branch',
    workspace_mode: 'git_worktree',
    setup_completed_at: null,
    created_at: '2026-04-18T00:00:00Z',
    updated_at: '2026-04-18T00:00:00Z',
    archived: false,
    pinned: false,
    name: 'Test workspace',
    worktree_deleted: false,
    ...overrides,
  };
}

function makeCapabilities(
  overrides: Partial<WorkspaceCapabilities> = {}
): WorkspaceCapabilities {
  return {
    supports_git_read: true,
    supports_git_write: true,
    supports_pull_requests: true,
    supports_repo_attach: true,
    supports_delete_branches: true,
    ...overrides,
  };
}

function makeBranchStatus(
  overrides: Partial<RepoBranchStatus> = {}
): RepoBranchStatus {
  return {
    repo_id: 'repo-1',
    repo_name: 'repo',
    commits_behind: 0,
    commits_ahead: 0,
    has_uncommitted_changes: false,
    head_oid: 'abc123',
    uncommitted_count: 0,
    untracked_count: 0,
    target_branch_name: 'main',
    remote_commits_behind: 0,
    remote_commits_ahead: 0,
    merges: [],
    is_rebase_in_progress: false,
    conflict_op: null,
    conflicted_files: [],
    is_target_remote: true,
    ...overrides,
  };
}

describe('loadDeleteWorkspaceDialogState', () => {
  it('skips branch status lookups when branch deletion is unsupported', async () => {
    const getBranchStatus = vi.fn<() => Promise<RepoBranchStatus[]>>();

    const state = await loadDeleteWorkspaceDialogState({
      getWorkspace: async () =>
        makeWorkspace({ workspace_mode: 'in_place_directory' }),
      getCapabilities: async () =>
        makeCapabilities({
          supports_git_read: false,
          supports_git_write: false,
          supports_pull_requests: false,
          supports_repo_attach: false,
          supports_delete_branches: false,
        }),
      getBranchStatus,
    });

    expect(getBranchStatus).not.toHaveBeenCalled();
    expect(state).toEqual({
      branchName: 'feature/test-branch',
      hasOpenPR: false,
      supportsDeleteBranches: false,
    });
  });

  it('loads branch status when branch deletion is supported', async () => {
    const getBranchStatus = vi.fn(async () => [
      makeBranchStatus({
        merges: [
          {
            type: 'pr',
            id: 'merge-1',
            workspace_id: 'workspace-1',
            repo_id: 'repo-1',
            created_at: '2026-04-18T00:00:00Z',
            target_branch_name: 'main',
            pr_info: {
              number: 42n,
              status: 'open',
              url: 'https://example.com/pr/42',
              merged_at: null,
              merge_commit_sha: null,
            },
          },
        ],
      }),
    ]);

    const state = await loadDeleteWorkspaceDialogState({
      getWorkspace: async () => makeWorkspace(),
      getCapabilities: async () => makeCapabilities(),
      getBranchStatus,
    });

    expect(getBranchStatus).toHaveBeenCalledOnce();
    expect(state).toEqual({
      branchName: 'feature/test-branch',
      hasOpenPR: true,
      supportsDeleteBranches: true,
    });
  });
});
