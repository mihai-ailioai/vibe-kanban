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

  it('persists directory sources in draft data for in_place_directory mode', () => {
    const draftData = toDraftWorkspaceData({
      initialPrompt: 'Inspect this folder',
      workspaceMode: 'in_place_directory',
      preferredRepos: null,
      workspaceSources: [
        {
          type: 'directory',
          path: '/Users/mihai/project',
          display_name: 'project',
        },
      ],
      linkedIssue: null,
      executorConfig: null,
    });

    expect(draftData).toMatchObject({
      message: 'Inspect this folder',
      workspace_mode: 'in_place_directory',
      sources: [
        {
          type: 'directory',
          path: '/Users/mihai/project',
          display_name: 'project',
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
      workspaceMode: 'git_worktree',
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

  it('builds a directory-mode create workspace request from canonical sources', () => {
    const executorConfig = {
      executor: 'CLAUDE_CODE',
      variant: 'DEFAULT',
    } as ExecutorConfig;

    const request = buildCreateWorkspaceRequest({
      name: 'Directory workspace',
      prompt: 'Work in place',
      executorConfig,
      workspaceMode: 'in_place_directory',
      sources: [
        {
          type: 'directory',
          path: '/Users/mihai/project',
          display_name: 'project',
        },
      ],
      linkedIssue: null,
      attachmentIds: [],
    });

    expect(request).toMatchObject({
      name: 'Directory workspace',
      prompt: 'Work in place',
      workspace_mode: 'in_place_directory',
      sources: [
        {
          type: 'directory',
          path: '/Users/mihai/project',
          display_name: 'project',
        },
      ],
      linked_issue: null,
      attachment_ids: [],
    });
    expect('repos' in request).toBe(false);
  });
});
