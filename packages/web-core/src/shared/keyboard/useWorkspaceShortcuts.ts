import { useCallback, useRef, useEffect } from 'react';
import { useHotkeys } from 'react-hotkeys-hook';
import { useActions } from '@/shared/hooks/useActions';
import { useWorkspaceContext } from '@/shared/hooks/useWorkspaceContext';
import { Actions } from '@/shared/actions';
import {
  type ActionDefinition,
  ActionTargetType,
} from '@/shared/types/actions';
import { Scope } from '@/shared/keyboard/registry';
import {
  areAppKeyboardShortcutsEnabled,
  withAppKeyboardCallbackGuard,
} from '@/shared/keyboard/shortcutGuards';

const SEQUENCE_TIMEOUT_MS = 1500;

const OPTIONS = {
  scopes: [Scope.WORKSPACE],
  enabled: (event: KeyboardEvent) => areAppKeyboardShortcutsEnabled(event),
  sequenceTimeout: SEQUENCE_TIMEOUT_MS,
} as const;

export function useWorkspaceShortcuts() {
  const { executeAction } = useActions();
  const { workspaceId, repos } = useWorkspaceContext();

  const workspaceIdRef = useRef(workspaceId);
  const reposRef = useRef(repos);
  const executeActionRef = useRef(executeAction);

  useEffect(() => {
    workspaceIdRef.current = workspaceId;
    reposRef.current = repos;
    executeActionRef.current = executeAction;
  });

  const execute = useCallback((action: ActionDefinition) => {
    const currentWorkspaceId = workspaceIdRef.current;
    const currentRepos = reposRef.current;
    const currentExecuteAction = executeActionRef.current;
    const firstRepoId = currentRepos?.[0]?.id;

    switch (action.requiresTarget) {
      case ActionTargetType.GIT:
        currentExecuteAction(action, currentWorkspaceId, firstRepoId);
        break;
      case ActionTargetType.WORKSPACE:
        currentExecuteAction(action, currentWorkspaceId);
        break;
      case ActionTargetType.NONE:
      case ActionTargetType.ISSUE:
        currentExecuteAction(action);
        break;
    }
  }, []);

  useHotkeys(
    'g>s',
    withAppKeyboardCallbackGuard(() => execute(Actions.Settings)),
    OPTIONS
  );
  useHotkeys(
    'g>n',
    withAppKeyboardCallbackGuard(() => execute(Actions.NewWorkspace)),
    OPTIONS
  );

  useHotkeys(
    'w>d',
    withAppKeyboardCallbackGuard(() => execute(Actions.DuplicateWorkspace)),
    OPTIONS
  );
  useHotkeys(
    'w>r',
    withAppKeyboardCallbackGuard(() => execute(Actions.RenameWorkspace)),
    OPTIONS
  );
  useHotkeys(
    'w>p',
    withAppKeyboardCallbackGuard(() => execute(Actions.PinWorkspace)),
    OPTIONS
  );
  useHotkeys(
    'w>a',
    withAppKeyboardCallbackGuard(() => execute(Actions.ArchiveWorkspace)),
    OPTIONS
  );
  useHotkeys(
    'w>x',
    withAppKeyboardCallbackGuard(() => execute(Actions.DeleteWorkspace)),
    OPTIONS
  );

  useHotkeys(
    'v>c',
    withAppKeyboardCallbackGuard(() => execute(Actions.ToggleChangesMode)),
    OPTIONS
  );
  useHotkeys(
    'v>l',
    withAppKeyboardCallbackGuard(() => execute(Actions.ToggleLogsMode)),
    OPTIONS
  );
  useHotkeys(
    'v>p',
    withAppKeyboardCallbackGuard(() => execute(Actions.TogglePreviewMode)),
    OPTIONS
  );
  useHotkeys(
    'v>s',
    withAppKeyboardCallbackGuard(() => execute(Actions.ToggleLeftSidebar)),
    OPTIONS
  );
  useHotkeys(
    'v>h',
    withAppKeyboardCallbackGuard(() => execute(Actions.ToggleLeftMainPanel)),
    OPTIONS
  );

  useHotkeys(
    'x>p',
    withAppKeyboardCallbackGuard(() => execute(Actions.GitCreatePR)),
    OPTIONS
  );
  useHotkeys(
    'x>m',
    withAppKeyboardCallbackGuard(() => execute(Actions.GitMerge)),
    OPTIONS
  );
  useHotkeys(
    'x>r',
    withAppKeyboardCallbackGuard(() => execute(Actions.GitRebase)),
    OPTIONS
  );
  useHotkeys(
    'x>u',
    withAppKeyboardCallbackGuard(() => execute(Actions.GitPush)),
    OPTIONS
  );

  useHotkeys(
    'y>p',
    withAppKeyboardCallbackGuard(() => execute(Actions.CopyWorkspacePath)),
    OPTIONS
  );
  useHotkeys(
    'y>l',
    withAppKeyboardCallbackGuard(() => execute(Actions.CopyRawLogs)),
    OPTIONS
  );

  useHotkeys(
    't>d',
    withAppKeyboardCallbackGuard(() => execute(Actions.ToggleDevServer)),
    OPTIONS
  );
  useHotkeys(
    't>w',
    withAppKeyboardCallbackGuard(() => execute(Actions.ToggleWrapLines)),
    OPTIONS
  );

  useHotkeys(
    'r>s',
    withAppKeyboardCallbackGuard(() => execute(Actions.RunSetupScript)),
    OPTIONS
  );
  useHotkeys(
    'r>c',
    withAppKeyboardCallbackGuard(() => execute(Actions.RunCleanupScript)),
    OPTIONS
  );
}
