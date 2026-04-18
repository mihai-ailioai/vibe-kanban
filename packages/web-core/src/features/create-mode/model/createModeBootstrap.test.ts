import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Repo } from 'shared/types';
import { resolveCreateModeBootstrap } from './createModeBootstrap';

const { getById } = vi.hoisted(() => ({
  getById: vi.fn(),
}));

vi.mock('@/shared/lib/api', () => ({
  repoApi: {
    getById,
  },
}));

describe('resolveCreateModeBootstrap', () => {
  beforeEach(() => {
    getById.mockReset();
  });

  it('restores repo selections from source-based scratch data', async () => {
    const repo = {
      id: 'repo-1',
      name: 'repo-1',
      display_name: 'Repo 1',
    } as Repo;
    getById.mockResolvedValue(repo);

    const result = await resolveCreateModeBootstrap({
      seedState: null,
      scratchData: {
        message: 'Resume draft',
        workspace_mode: 'git_worktree',
        sources: [
          {
            type: 'git_repo',
            repo_id: 'repo-1',
            target_branch: 'main',
          },
        ],
        executor_config: null,
        linked_issue: null,
        attachments: [],
      },
      defaultExecutorConfig: null,
      isValidProfile: () => true,
    });

    expect(result.source).toBe('scratch');
    expect(result.data.repos).toEqual([
      {
        repo,
        targetBranch: 'main',
      },
    ]);
    expect(result.data.workspaceMode).toBe('git_worktree');
  });

  it('preserves seed workspace mode without requiring repo defaults', async () => {
    const result = await resolveCreateModeBootstrap({
      seedState: {
        workspaceMode: 'in_place_directory',
      },
      defaultExecutorConfig: null,
      isValidProfile: () => true,
    });

    expect(result).toEqual({
      source: 'seed',
      data: {
        workspaceMode: 'in_place_directory',
      },
    });
  });

  it('restores directory sources from scratch data for in_place_directory mode', async () => {
    const result = await resolveCreateModeBootstrap({
      seedState: null,
      scratchData: {
        message: 'Resume directory draft',
        workspace_mode: 'in_place_directory',
        sources: [
          {
            type: 'directory',
            path: '/Users/mihai/project',
            display_name: 'project',
          },
        ],
        executor_config: null,
        linked_issue: null,
        attachments: [],
      },
      defaultExecutorConfig: null,
      isValidProfile: () => true,
    });

    expect(result).toEqual({
      source: 'scratch',
      data: {
        message: 'Resume directory draft',
        workspaceMode: 'in_place_directory',
        directorySource: {
          type: 'directory',
          path: '/Users/mihai/project',
          display_name: 'project',
        },
      },
    });
  });
});
