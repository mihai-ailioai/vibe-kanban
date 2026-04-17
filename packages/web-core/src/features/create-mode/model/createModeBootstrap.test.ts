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
  });
});
