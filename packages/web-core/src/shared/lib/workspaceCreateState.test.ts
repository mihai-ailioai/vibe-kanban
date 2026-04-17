import { describe, expect, it } from 'vitest';
import type { ExecutorConfig } from 'shared/types';
import {
  buildCreateWorkspaceRequest,
  buildWorkspaceCreateInitialState,
  toDraftWorkspaceData,
} from './workspaceCreateState';

describe('workspaceCreateState', () => {
  it('defaults create-mode initial state to git_worktree', () => {
    const initialState = buildWorkspaceCreateInitialState({
      prompt: 'Ship it',
    });

    expect(initialState.workspaceMode).toBe('git_worktree');
  });

  it('persists selected repos as canonical git sources in draft data', () => {
    const draftData = toDraftWorkspaceData({
      initialPrompt: 'Implement Task 6',
      workspaceMode: 'git_worktree',
      preferredRepos: [
        {
          repo_id: 'repo-1',
          target_branch: 'main',
        },
      ],
      linkedIssue: {
        issueId: 'issue-1',
        remoteProjectId: 'project-1',
      },
      executorConfig: null,
    });

    expect(draftData).toMatchObject({
      message: 'Implement Task 6',
      workspace_mode: 'git_worktree',
      sources: [
        {
          type: 'git_repo',
          repo_id: 'repo-1',
          target_branch: 'main',
        },
      ],
    });
    expect('repos' in draftData).toBe(false);
  });

  it('builds a strict source-only create workspace request for selected repos', () => {
    const executorConfig = {
      executor: 'CLAUDE_CODE',
      variant: 'DEFAULT',
    } as ExecutorConfig;

    const request = buildCreateWorkspaceRequest({
      name: 'Task 6',
      prompt: 'Implement the contract migration',
      executorConfig,
      repos: [
        {
          repo_id: 'repo-1',
          target_branch: 'main',
        },
        {
          repo_id: 'repo-2',
          target_branch: 'develop',
        },
      ],
      linkedIssue: {
        issueId: 'issue-1',
        remoteProjectId: 'project-1',
      },
      attachmentIds: ['attachment-1'],
    });

    expect(request).toMatchObject({
      name: 'Task 6',
      prompt: 'Implement the contract migration',
      workspace_mode: 'git_worktree',
      sources: [
        {
          type: 'git_repo',
          repo_id: 'repo-1',
          target_branch: 'main',
        },
        {
          type: 'git_repo',
          repo_id: 'repo-2',
          target_branch: 'develop',
        },
      ],
      linked_issue: {
        issue_id: 'issue-1',
        remote_project_id: 'project-1',
      },
      attachment_ids: ['attachment-1'],
    });
    expect('repos' in request).toBe(false);
  });
});
