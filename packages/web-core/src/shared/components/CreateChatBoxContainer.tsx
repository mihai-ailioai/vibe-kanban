import { useMemo, useCallback, useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { useDropzone } from 'react-dropzone';
import { useCreateMode } from '@/features/create-mode/model/useCreateMode';
import { AgentIcon } from '@/shared/components/AgentIcon';
import { useUserSystem } from '@/shared/hooks/useUserSystem';
import WYSIWYGEditor from '@/shared/components/WYSIWYGEditor';
import { useCreateWorkspace } from '@/shared/hooks/useCreateWorkspace';
import { useCreateAttachments } from '@/shared/hooks/useCreateAttachments';
import { useExecutorConfig } from '@/shared/hooks/useExecutorConfig';
import { saveProjectRepoDefaults } from '@/shared/hooks/useProjectRepoDefaults';
import {
  buildCreateWorkspaceRequest,
  buildWorkspaceSourcesForMode,
} from '@/shared/lib/workspaceCreateState';
import {
  toPrettyCase,
  splitMessageToTitleDescription,
} from '@/shared/lib/string';
import type { Repo } from 'shared/types';
import { CreateChatBox } from '@vibe/ui/components/CreateChatBox';
import { SettingsDialog } from '@/shared/dialogs/settings/SettingsDialog';
import { CreateModeRepoPickerBar } from './CreateModeRepoPickerBar';
import { ModelSelectorContainer } from '@/shared/components/ModelSelectorContainer';

function getRepoDisplayName(repo: Repo) {
  return repo.display_name || repo.name;
}

const BRANCH_LABEL_MAX_CHARS = 15;

function truncateBranchLabel(branch: string) {
  return branch.length > BRANCH_LABEL_MAX_CHARS
    ? `${branch.slice(0, BRANCH_LABEL_MAX_CHARS)}...`
    : branch;
}

interface CreateChatBoxContainerProps {
  onWorkspaceCreated: (workspaceId: string) => void;
}

export function CreateChatBoxContainer({
  onWorkspaceCreated,
}: CreateChatBoxContainerProps) {
  const { t } = useTranslation('common');
  const { profiles, config } = useUserSystem();
  const {
    workspaceMode,
    directorySource,
    repos,
    targetBranches,
    message,
    setMessage,
    setWorkspaceMode,
    clearDraft,
    hasInitialValue,
    hasResolvedInitialRepoDefaults,
    linkedIssue,
    clearLinkedIssue,
    preferredExecutorConfig,
    executorConfig: draftConfig,
    setExecutorConfig: setDraftConfig,
    attachments: draftAttachments,
    setAttachments: setDraftAttachments,
  } = useCreateMode();

  const { createWorkspace } = useCreateWorkspace();
  const hasSelectedRepos = repos.length > 0;
  const hasSelectedSource =
    workspaceMode === 'in_place_directory'
      ? !!directorySource
      : hasSelectedRepos;
  const [hasAttemptedSubmit, setHasAttemptedSubmit] = useState(false);
  const [hasInitializedStep, setHasInitializedStep] = useState(false);
  const [isSelectingRepos, setIsSelectingRepos] = useState(true);

  useEffect(() => {
    if (!hasInitialValue || hasInitializedStep) return;
    if (workspaceMode !== 'in_place_directory') {
      if (!hasSelectedRepos && !hasResolvedInitialRepoDefaults) return;
    }

    setIsSelectingRepos(!hasSelectedSource);
    setHasInitializedStep(true);
  }, [
    hasInitialValue,
    hasInitializedStep,
    workspaceMode,
    hasSelectedSource,
    hasSelectedRepos,
    hasResolvedInitialRepoDefaults,
  ]);

  const showRepoPickerStep = !hasSelectedSource || isSelectingRepos;
  const showChatStep = hasSelectedSource && !isSelectingRepos;

  // Attachment handling - insert markdown and track attachment IDs
  const handleInsertMarkdown = useCallback(
    (markdown: string) => {
      const newMessage = message.trim()
        ? `${message}\n\n${markdown}`
        : markdown;
      setMessage(newMessage);
    },
    [message, setMessage]
  );

  const { uploadFiles, getAttachmentIds, clearAttachments, localAttachments } =
    useCreateAttachments(
      handleInsertMarkdown,
      draftAttachments,
      setDraftAttachments
    );

  const onDrop = useCallback(
    (acceptedFiles: File[]) => {
      if (acceptedFiles.length > 0) {
        uploadFiles(acceptedFiles);
      }
    },
    [uploadFiles]
  );

  const { getRootProps, getInputProps, isDragActive } = useDropzone({
    onDrop,
    disabled: createWorkspace.isPending || !hasSelectedSource,
    noClick: true,
    noKeyboard: true,
  });

  const scratchConfig = useMemo(() => {
    if (!hasInitialValue) return undefined; // still loading
    return draftConfig ?? null;
  }, [hasInitialValue, draftConfig]);

  const {
    executorConfig,
    effectiveExecutor,
    selectedVariant,
    executorOptions,
    variantOptions,
    presetOptions,
    setExecutor: handleExecutorChange,
    setVariant: handlePresetSelect,
    setOverrides: setExecutorOverrides,
  } = useExecutorConfig({
    profiles,
    lastUsedConfig: preferredExecutorConfig,
    scratchConfig,
    configExecutorProfile: config?.executor_profile,
    onPersist: (cfg) => setDraftConfig(cfg),
  });

  const repoId = repos.length === 1 ? repos[0]?.id : undefined;
  const repoSummaryLabel = useMemo(() => {
    if (workspaceMode === 'in_place_directory') {
      return (
        directorySource?.display_name ||
        directorySource?.path ||
        'Select folder'
      );
    }

    if (repos.length === 1) {
      const repo = repos[0];
      if (!repo) return '0 repositories selected';
      const selectedBranch = targetBranches[repo.id];
      const branch = selectedBranch
        ? truncateBranchLabel(selectedBranch)
        : 'Select branch';
      return `${getRepoDisplayName(repo)} · ${branch}`;
    }

    return `${repos.length} repositories selected`;
  }, [directorySource, repos, targetBranches, workspaceMode]);

  const repoSummaryTitle = useMemo(
    () =>
      workspaceMode === 'in_place_directory'
        ? directorySource?.path || 'Select folder'
        : repos
            .map((repo) => {
              const branch = targetBranches[repo.id] ?? 'Select branch';
              return `${getRepoDisplayName(repo)} (${branch})`;
            })
            .join('\n'),
    [directorySource, repos, targetBranches, workspaceMode]
  );

  const hasSelectedBranchesForAllRepos = repos.every(
    (repo) => !!targetBranches[repo.id]
  );

  // Determine if we can submit
  const canSubmit =
    hasSelectedSource &&
    (workspaceMode === 'in_place_directory' ||
      hasSelectedBranchesForAllRepos) &&
    message.trim().length > 0 &&
    effectiveExecutor !== null;

  const modeWarning =
    workspaceMode === 'in_place_git'
      ? 'Changes will be made directly in the selected repository. Use Git worktree if you want isolation.'
      : workspaceMode === 'in_place_directory'
        ? 'This workspace runs directly in the selected folder. Git actions and pull request flows will be unavailable.'
        : null;

  const handleCustomise = () => {
    SettingsDialog.show({ initialSection: 'agents' });
  };

  // Handle submit
  const handleSubmit = useCallback(async () => {
    setHasAttemptedSubmit(true);
    if (!canSubmit || !executorConfig) return;

    const { title } = splitMessageToTitleDescription(message);
    const selectedRepos = repos.map((r) => ({
      repo_id: r.id,
      target_branch: targetBranches[r.id]!,
    }));
    const sources = buildWorkspaceSourcesForMode({
      workspaceMode,
      repos: selectedRepos,
      directorySource,
    });
    const data = buildCreateWorkspaceRequest({
      name: title,
      prompt: message,
      executorConfig,
      workspaceMode,
      sources,
      linkedIssue: linkedIssue
        ? {
            remoteProjectId: linkedIssue.remoteProjectId,
            issueId: linkedIssue.issueId,
          }
        : null,
      attachmentIds: getAttachmentIds() ?? [],
    });
    const linkToIssue = linkedIssue
      ? {
          remoteProjectId: linkedIssue.remoteProjectId,
          issueId: linkedIssue.issueId,
        }
      : undefined;

    const result = await createWorkspace.mutateAsync({
      data,
      linkToIssue,
    });

    if (result.workspace) {
      onWorkspaceCreated(result.workspace.id);
    }

    if (linkedIssue?.remoteProjectId) {
      saveProjectRepoDefaults(linkedIssue.remoteProjectId, selectedRepos).catch(
        (err) => console.warn('Failed to save project repo defaults:', err)
      );
    }

    clearAttachments();
    await clearDraft();
  }, [
    canSubmit,
    directorySource,
    executorConfig,
    message,
    repos,
    workspaceMode,
    targetBranches,
    createWorkspace,
    onWorkspaceCreated,
    getAttachmentIds,
    clearAttachments,
    clearDraft,
    linkedIssue,
  ]);

  // Determine error to display
  const displayError =
    hasAttemptedSubmit && !hasSelectedSource
      ? workspaceMode === 'in_place_directory'
        ? 'Select a folder before creating a workspace'
        : 'Add at least one repository to create a workspace'
      : hasAttemptedSubmit &&
          workspaceMode !== 'in_place_directory' &&
          !hasSelectedBranchesForAllRepos
        ? 'Select a branch for every repository before creating a workspace'
        : createWorkspace.error
          ? createWorkspace.error instanceof Error
            ? createWorkspace.error.message
            : 'Failed to create workspace'
          : null;

  // Wait for initial value to be applied before rendering
  // This ensures the editor mounts with content ready, so autoFocus works correctly
  if (!hasInitialValue) {
    return null;
  }

  return (
    <div className="relative flex flex-1 flex-col bg-primary h-full">
      <div className="flex flex-1 items-center justify-center px-base">
        <div className="flex w-chat max-w-full flex-col gap-base">
          {showRepoPickerStep && (
            <>
              <h2 className="mb-double text-center text-4xl font-medium tracking-tight text-high">
                {t('createMode.headings.repoStep')}
              </h2>
              <CreateModeRepoPickerBar
                workspaceMode={workspaceMode}
                onWorkspaceModeChange={setWorkspaceMode}
                onContinueToPrompt={() => setIsSelectingRepos(false)}
              />
            </>
          )}

          {showChatStep && (
            <>
              <h2 className="mb-double text-center text-4xl font-medium tracking-tight text-high">
                {t('createMode.headings.chatStep')}
              </h2>

              <div className="flex justify-center @container">
                <div className="flex w-full flex-col gap-half">
                  {modeWarning && (
                    <div className="rounded-sm border border-brand/20 bg-brand/5 px-base py-half text-sm text-normal">
                      {modeWarning}
                    </div>
                  )}
                  <CreateChatBox
                    editor={{
                      value: message,
                      onChange: setMessage,
                    }}
                    renderEditor={({
                      value,
                      onChange,
                      onCmdEnter,
                      disabled,
                      repoIds,
                      repoId,
                      executor,
                      onPasteFiles,
                      localAttachments,
                    }) => (
                      <WYSIWYGEditor
                        placeholder="Describe the task..."
                        value={value}
                        onChange={onChange}
                        onCmdEnter={onCmdEnter}
                        disabled={disabled}
                        className="min-h-double max-h-[50vh] overflow-y-auto"
                        repoIds={repoIds}
                        repoId={repoId}
                        executor={executor}
                        autoFocus
                        onPasteFiles={onPasteFiles}
                        localAttachments={localAttachments}
                        sendShortcut={config?.send_message_shortcut}
                      />
                    )}
                    agentIcon={
                      <AgentIcon
                        agent={effectiveExecutor}
                        className="size-icon-xl"
                      />
                    }
                    onSend={handleSubmit}
                    isSending={createWorkspace.isPending}
                    disabled={!hasSelectedSource}
                    executor={{
                      selected: effectiveExecutor,
                      options: executorOptions,
                      onChange: handleExecutorChange,
                    }}
                    formatExecutorLabel={toPrettyCase}
                    error={displayError}
                    repoIds={
                      workspaceMode === 'in_place_directory'
                        ? []
                        : repos.map((r) => r.id)
                    }
                    repoId={
                      workspaceMode === 'in_place_directory'
                        ? undefined
                        : repoId
                    }
                    modelSelector={
                      effectiveExecutor ? (
                        <ModelSelectorContainer
                          agent={effectiveExecutor}
                          workspaceId={undefined}
                          onAdvancedSettings={handleCustomise}
                          presets={variantOptions}
                          selectedPreset={selectedVariant}
                          onPresetSelect={handlePresetSelect}
                          onOverrideChange={setExecutorOverrides}
                          executorConfig={executorConfig}
                          presetOptions={presetOptions}
                        />
                      ) : undefined
                    }
                    onPasteFiles={uploadFiles}
                    localAttachments={localAttachments}
                    dropzone={{ getRootProps, getInputProps, isDragActive }}
                    onEditRepos={() => setIsSelectingRepos(true)}
                    repoSummaryLabel={repoSummaryLabel}
                    repoSummaryTitle={repoSummaryTitle}
                    linkedIssue={
                      linkedIssue?.simpleId
                        ? {
                            simpleId: linkedIssue.simpleId,
                            title: linkedIssue.title ?? '',
                            onRemove: clearLinkedIssue,
                          }
                        : null
                    }
                  />
                </div>
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
