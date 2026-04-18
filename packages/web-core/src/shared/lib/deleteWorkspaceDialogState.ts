import type { RepoBranchStatus, WorkspaceCapabilities } from 'shared/types';

export type DeleteWorkspaceDialogState = {
  branchName: string;
  hasOpenPR: boolean;
  supportsDeleteBranches: boolean;
};

type LoadDeleteWorkspaceDialogStateParams = {
  getWorkspace: () => Promise<{ branch: string }>;
  getCapabilities: () => Promise<WorkspaceCapabilities>;
  getBranchStatus: () => Promise<RepoBranchStatus[]>;
};

function hasOpenPullRequest(branchStatus: RepoBranchStatus[]): boolean {
  return branchStatus.some((repoStatus) =>
    repoStatus.merges?.some(
      (merge) => merge.type === 'pr' && merge.pr_info.status === 'open'
    )
  );
}

export async function loadDeleteWorkspaceDialogState({
  getWorkspace,
  getCapabilities,
  getBranchStatus,
}: LoadDeleteWorkspaceDialogStateParams): Promise<DeleteWorkspaceDialogState> {
  const [workspace, capabilities] = await Promise.all([
    getWorkspace(),
    getCapabilities(),
  ]);

  if (!capabilities.supports_delete_branches) {
    return {
      branchName: workspace.branch,
      hasOpenPR: false,
      supportsDeleteBranches: false,
    };
  }

  const branchStatus = await getBranchStatus();

  return {
    branchName: workspace.branch,
    hasOpenPR: hasOpenPullRequest(branchStatus),
    supportsDeleteBranches: true,
  };
}
