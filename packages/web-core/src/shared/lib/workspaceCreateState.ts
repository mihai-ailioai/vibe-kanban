import type { CreateModeInitialState } from '@/shared/types/createMode';
import type {
  CreateAndStartWorkspaceRequest,
  DraftWorkspaceData,
  ExecutorConfig,
  WorkspaceMode,
  WorkspaceSourceInput,
} from 'shared/types';
import { ScratchType } from 'shared/types';
import type { AppRuntime } from '@/shared/hooks/useAppRuntime';
import { scratchApi } from '@/shared/lib/api';
import { localStorageScratchUpdate } from '@/shared/hooks/useLocalStorageScratch';

interface WorkspaceDefaultsLike {
  preferredRepos?: CreateModeInitialState['preferredRepos'];
  project_id?: string | null;
}

interface LocalWorkspaceLike {
  id: string;
}

interface LinkedIssueSource {
  id: string;
  simple_id: string;
  title: string;
}

export interface RepoSelectionInput {
  repo_id: string;
  target_branch: string;
}

export type DirectoryWorkspaceSourceInput = Extract<
  WorkspaceSourceInput,
  { type: 'directory' }
>;

interface LinkedIssueRequestInput {
  issueId: string;
  remoteProjectId: string;
}

export const DEFAULT_WORKSPACE_CREATE_DRAFT_ID =
  '00000000-0000-0000-0000-000000000001';

export const DEFAULT_CREATE_MODE_WORKSPACE_MODE: WorkspaceMode = 'git_worktree';

export function toGitRepoWorkspaceSources(
  repos: RepoSelectionInput[]
): Extract<WorkspaceSourceInput, { type: 'git_repo' }>[] {
  return repos.map((repo) => ({
    type: 'git_repo',
    repo_id: repo.repo_id,
    target_branch: repo.target_branch,
  }));
}

export function buildWorkspaceSourcesForMode(args: {
  workspaceMode: WorkspaceMode;
  repos?: RepoSelectionInput[];
  directorySource?: DirectoryWorkspaceSourceInput | null;
  workspaceSources?: WorkspaceSourceInput[] | null;
}): WorkspaceSourceInput[] {
  if (args.workspaceSources) {
    return args.workspaceSources;
  }

  if (args.workspaceMode === 'in_place_directory') {
    return args.directorySource ? [args.directorySource] : [];
  }

  return toGitRepoWorkspaceSources(args.repos ?? []);
}

export function buildWorkspaceCreatePrompt(
  title: string | null | undefined,
  description: string | null | undefined
): string | null {
  const trimmedTitle = title?.trim();
  if (!trimmedTitle) return null;

  const trimmedDescription = description?.trim();
  return trimmedDescription
    ? `${trimmedTitle}\n\n${trimmedDescription}`
    : trimmedTitle;
}

export function buildLinkedIssueCreateState(
  issue: LinkedIssueSource | null | undefined,
  projectId: string
): NonNullable<CreateModeInitialState['linkedIssue']> | null {
  if (!issue) return null;
  return {
    issueId: issue.id,
    simpleId: issue.simple_id,
    title: issue.title,
    remoteProjectId: projectId,
  };
}

export function buildWorkspaceCreateInitialState(args: {
  prompt: string | null;
  defaults?: WorkspaceDefaultsLike | null;
  linkedIssue?: CreateModeInitialState['linkedIssue'];
  executorConfig?: CreateModeInitialState['executorConfig'];
}): CreateModeInitialState {
  return {
    initialPrompt: args.prompt,
    workspaceMode: DEFAULT_CREATE_MODE_WORKSPACE_MODE,
    workspaceSources: toGitRepoWorkspaceSources(
      args.defaults?.preferredRepos?.map((repo) => ({
        repo_id: repo.repo_id,
        target_branch: repo.target_branch ?? '',
      })) ?? []
    ),
    preferredRepos: args.defaults?.preferredRepos ?? null,
    project_id: args.defaults?.project_id ?? null,
    linkedIssue: args.linkedIssue ?? null,
    executorConfig: args.executorConfig ?? null,
  };
}

export function buildLocalWorkspaceIdSet(
  activeWorkspaces: LocalWorkspaceLike[],
  archivedWorkspaces: LocalWorkspaceLike[]
): Set<string> {
  return new Set([
    ...activeWorkspaces.map((workspace) => workspace.id),
    ...archivedWorkspaces.map((workspace) => workspace.id),
  ]);
}

export function toDraftWorkspaceData(
  initialState: CreateModeInitialState
): DraftWorkspaceData {
  const workspaceMode =
    initialState.workspaceMode ?? DEFAULT_CREATE_MODE_WORKSPACE_MODE;

  return {
    message: initialState.initialPrompt ?? '',
    workspace_mode: workspaceMode,
    sources: buildWorkspaceSourcesForMode({
      workspaceMode,
      repos:
        initialState.preferredRepos?.map((repo) => ({
          repo_id: repo.repo_id,
          target_branch: repo.target_branch ?? '',
        })) ?? [],
      workspaceSources: initialState.workspaceSources ?? null,
    }),
    executor_config: initialState.executorConfig ?? null,
    linked_issue: initialState.linkedIssue
      ? {
          issue_id: initialState.linkedIssue.issueId,
          simple_id: initialState.linkedIssue.simpleId ?? '',
          title: initialState.linkedIssue.title ?? '',
          remote_project_id: initialState.linkedIssue.remoteProjectId,
        }
      : null,
    attachments: [],
  };
}

export function buildCreateWorkspaceRequest(args: {
  name: string | null;
  prompt: string;
  executorConfig: ExecutorConfig;
  workspaceMode: WorkspaceMode;
  sources: WorkspaceSourceInput[];
  linkedIssue?: LinkedIssueRequestInput | null;
  attachmentIds: string[];
}): CreateAndStartWorkspaceRequest {
  return {
    name: args.name,
    prompt: args.prompt,
    executor_config: args.executorConfig,
    workspace_mode: args.workspaceMode,
    sources: args.sources,
    linked_issue: args.linkedIssue
      ? {
          remote_project_id: args.linkedIssue.remoteProjectId,
          issue_id: args.linkedIssue.issueId,
        }
      : null,
    attachment_ids: args.attachmentIds,
  };
}

export async function persistWorkspaceCreateDraft(
  initialState: CreateModeInitialState,
  draftId = DEFAULT_WORKSPACE_CREATE_DRAFT_ID,
  runtime: AppRuntime = 'local'
): Promise<string | null> {
  const draftData = toDraftWorkspaceData(initialState);
  const payload = {
    type: 'DRAFT_WORKSPACE' as const,
    data: draftData,
  };

  try {
    if (runtime === 'remote') {
      const didPersist = localStorageScratchUpdate(
        ScratchType.DRAFT_WORKSPACE,
        draftId,
        {
          payload,
        }
      );
      if (!didPersist) {
        throw new Error('Failed to persist create-workspace draft in storage');
      }
    } else {
      await scratchApi.update(ScratchType.DRAFT_WORKSPACE, draftId, {
        payload,
      });
    }
    return draftId;
  } catch (error) {
    console.error('Failed to persist create-workspace draft:', error);
    return null;
  }
}
