export const TERMINAL_SHORTCUTS_ROOT_ATTR = 'data-terminal-shortcuts-root';

type KeyboardEventLike = {
  target?: EventTarget | null;
};

type KeyboardTargetLike = {
  tagName?: string;
  isContentEditable?: boolean;
  parentElement?: KeyboardTargetLike | null;
  hasAttribute?: (name: string) => boolean;
};

function isStandardEditableTarget(target: KeyboardTargetLike | null): boolean {
  if (!target) return false;

  const tagName = target.tagName?.toUpperCase();

  return (
    tagName === 'INPUT' ||
    tagName === 'TEXTAREA' ||
    target.isContentEditable === true
  );
}

function isInsideTerminalShortcutBoundary(
  target: KeyboardTargetLike | null
): boolean {
  let current = target;

  while (current) {
    if (current.hasAttribute?.(TERMINAL_SHORTCUTS_ROOT_ATTR)) {
      return true;
    }

    current = current.parentElement ?? null;
  }

  return false;
}

function getKeyboardTarget(
  targetOrEvent?: EventTarget | KeyboardEventLike | null
): EventTarget | null {
  if (
    targetOrEvent &&
    typeof targetOrEvent === 'object' &&
    'target' in targetOrEvent
  ) {
    return targetOrEvent.target ?? null;
  }

  return (targetOrEvent as EventTarget | null | undefined) ?? null;
}

export function shouldIgnoreAppKeyboardTarget(target: EventTarget | null) {
  const element = target as KeyboardTargetLike | null;

  return (
    isStandardEditableTarget(element) ||
    isInsideTerminalShortcutBoundary(element)
  );
}

export function areAppKeyboardShortcutsEnabled(
  targetOrEvent?: EventTarget | KeyboardEventLike | null
): boolean {
  const liveTarget = getKeyboardTarget(targetOrEvent);

  if (liveTarget) {
    return !shouldIgnoreAppKeyboardTarget(liveTarget);
  }

  if (typeof document === 'undefined') {
    return true;
  }

  return !shouldIgnoreAppKeyboardTarget(document.activeElement);
}

export function withAppKeyboardTargetGuard(
  enabled: boolean | (() => boolean)
): boolean | (() => boolean) {
  if (typeof enabled === 'function') {
    return (event?: KeyboardEvent) =>
      enabled() && areAppKeyboardShortcutsEnabled(event);
  }

  if (!enabled) {
    return false;
  }

  return (event?: KeyboardEvent) => areAppKeyboardShortcutsEnabled(event);
}

export function withAppKeyboardCallbackGuard<TArgs extends unknown[], TReturn>(
  callback: (event?: KeyboardEvent, ...args: TArgs) => TReturn
) {
  return (event?: KeyboardEvent, ...args: TArgs): TReturn | undefined => {
    if (!areAppKeyboardShortcutsEnabled(event)) {
      return;
    }

    return callback(event, ...args);
  };
}
