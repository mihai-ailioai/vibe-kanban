import type {
  DraftWorkspaceData,
  DraftWorkspaceAttachment,
  ExecutorConfig,
  Repo,
  WorkspaceMode,
  WorkspaceSourceInput,
} from 'shared/types';
import { repoApi } from '@/shared/lib/api';
import type {
  CreateModeInitialState,
  LinkedIssue,
} from '@/shared/types/createMode';

export interface BootstrapSelectedRepo {
  repo: Repo;
  targetBranch: string | null;
}

export interface CreateModeBootstrapData {
  message?: string;
  workspaceMode?: WorkspaceMode;
  linkedIssue?: LinkedIssue | null;
  repos?: BootstrapSelectedRepo[];
  directorySource?: Extract<WorkspaceSourceInput, { type: 'directory' }> | null;
  executorConfig?: ExecutorConfig | null;
  attachments?: DraftWorkspaceAttachment[];
}

export interface ResolveCreateModeBootstrapParams {
  seedState: CreateModeInitialState | null;
  scratchData?: DraftWorkspaceData;
  defaultExecutorConfig?: ExecutorConfig | null;
  isValidProfile: (config: ExecutorConfig | null) => boolean;
}

export interface ResolveCreateModeBootstrapResult {
  source: 'seed' | 'scratch' | 'fresh';
  data: CreateModeBootstrapData;
}

interface PreferredRepoInput {
  repo_id: string;
  target_branch: string | null;
}

function getScratchGitRepos(
  scratchData: DraftWorkspaceData
): PreferredRepoInput[] {
  if ('sources' in scratchData) {
    return (
      scratchData.sources
        ?.filter(
          (source): source is Extract<typeof source, { type: 'git_repo' }> =>
            source.type === 'git_repo'
        )
        .map((source) => ({
          repo_id: source.repo_id,
          target_branch: source.target_branch ?? null,
        })) ?? []
    );
  }

  if ('repos' in scratchData) {
    return scratchData.repos.map((repo) => ({
      repo_id: repo.repo_id,
      target_branch: repo.target_branch ?? null,
    }));
  }

  return [];
}

function getGitReposFromSources(
  sources: WorkspaceSourceInput[] | null | undefined
): PreferredRepoInput[] {
  return (
    sources
      ?.filter(
        (
          source
        ): source is Extract<WorkspaceSourceInput, { type: 'git_repo' }> =>
          source.type === 'git_repo'
      )
      .map((source) => ({
        repo_id: source.repo_id,
        target_branch: source.target_branch ?? null,
      })) ?? []
  );
}

function getDirectorySource(
  sources: WorkspaceSourceInput[] | null | undefined
): Extract<WorkspaceSourceInput, { type: 'directory' }> | null {
  return (
    sources?.find(
      (
        source
      ): source is Extract<WorkspaceSourceInput, { type: 'directory' }> =>
        source.type === 'directory'
    ) ?? null
  );
}

export async function resolveBootstrapRepos(
  preferredRepos: PreferredRepoInput[]
): Promise<BootstrapSelectedRepo[]> {
  const reposById = new Map<string, Repo>();

  const missingRepoIds = preferredRepos
    .map((repo) => repo.repo_id)
    .filter((repoId) => !reposById.has(repoId));

  if (missingRepoIds.length > 0) {
    const fetchedRepos = await Promise.all(
      missingRepoIds.map(async (repoId) => {
        try {
          return await repoApi.getById(repoId);
        } catch {
          return null;
        }
      })
    );

    for (const repo of fetchedRepos) {
      if (repo) {
        reposById.set(repo.id, repo);
      }
    }
  }

  return preferredRepos.flatMap((preferredRepo) => {
    const repo = reposById.get(preferredRepo.repo_id);
    if (!repo) return [];

    return [
      {
        repo,
        targetBranch: preferredRepo.target_branch ?? null,
      },
    ];
  });
}

export async function resolveCreateModeBootstrap({
  seedState,
  scratchData,
  defaultExecutorConfig,
  isValidProfile,
}: ResolveCreateModeBootstrapParams): Promise<ResolveCreateModeBootstrapResult> {
  const hasInitialPrompt = !!seedState?.initialPrompt;
  const hasWorkspaceMode = !!seedState?.workspaceMode;
  const hasLinkedIssue = !!seedState?.linkedIssue;
  const hasPreferredRepos = (seedState?.preferredRepos?.length ?? 0) > 0;
  const hasWorkspaceSources = (seedState?.workspaceSources?.length ?? 0) > 0;
  const hasExecutorConfig = !!seedState?.executorConfig;

  if (
    hasInitialPrompt ||
    hasWorkspaceMode ||
    hasLinkedIssue ||
    hasPreferredRepos ||
    hasWorkspaceSources ||
    hasExecutorConfig
  ) {
    const data: CreateModeBootstrapData = {};
    let appliedSeedState = false;

    if (hasInitialPrompt) {
      data.message = seedState!.initialPrompt!;
      appliedSeedState = true;
    }

    if (hasWorkspaceMode) {
      data.workspaceMode = seedState!.workspaceMode!;
      appliedSeedState = true;
    }

    if (hasLinkedIssue) {
      data.linkedIssue = seedState!.linkedIssue!;
      appliedSeedState = true;
    }

    const seedGitRepos = seedState?.workspaceSources?.length
      ? getGitReposFromSources(seedState.workspaceSources)
      : (seedState?.preferredRepos ?? []);

    if (seedGitRepos.length > 0) {
      const resolvedRepos = await resolveBootstrapRepos(seedGitRepos);
      if (resolvedRepos.length > 0) {
        data.repos = resolvedRepos;
        appliedSeedState = true;
      }
    }

    if (seedState?.workspaceSources?.length) {
      const directorySource = getDirectorySource(seedState.workspaceSources);
      if (directorySource) {
        data.directorySource = directorySource;
        appliedSeedState = true;
      }
    }

    if (seedState?.executorConfig && isValidProfile(seedState.executorConfig)) {
      data.executorConfig = seedState.executorConfig;
      appliedSeedState = true;
    }

    if (appliedSeedState) {
      return {
        source: 'seed',
        data,
      };
    }
  }

  if (scratchData) {
    const data: CreateModeBootstrapData = {};

    data.workspaceMode =
      ('workspace_mode' in scratchData
        ? scratchData.workspace_mode
        : undefined) ?? 'git_worktree';

    if (scratchData.message) {
      data.message = scratchData.message;
    }

    if (
      scratchData.executor_config &&
      isValidProfile(scratchData.executor_config)
    ) {
      data.executorConfig = scratchData.executor_config;
    }

    if (scratchData.linked_issue) {
      data.linkedIssue = {
        issueId: scratchData.linked_issue.issue_id,
        simpleId: scratchData.linked_issue.simple_id || undefined,
        title: scratchData.linked_issue.title || undefined,
        remoteProjectId: scratchData.linked_issue.remote_project_id,
      };
    }

    if (scratchData.attachments?.length > 0) {
      data.attachments = scratchData.attachments;
    }

    if ('sources' in scratchData) {
      const directorySource = getDirectorySource(scratchData.sources);
      if (directorySource) {
        data.directorySource = directorySource;
      }
    }

    const scratchGitRepos = getScratchGitRepos(scratchData);

    if (scratchGitRepos?.length > 0) {
      const restoredRepos = await resolveBootstrapRepos(scratchGitRepos);

      if (restoredRepos.length > 0) {
        data.repos = restoredRepos;
      }
    }

    return {
      source: 'scratch',
      data,
    };
  }

  return {
    source: 'fresh',
    data:
      defaultExecutorConfig && isValidProfile(defaultExecutorConfig)
        ? { executorConfig: defaultExecutorConfig }
        : {},
  };
}
