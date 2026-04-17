import { describe, expect, it, vi } from 'vitest';

import {
  areAppKeyboardShortcutsEnabled,
  withAppKeyboardCallbackGuard,
  shouldIgnoreAppKeyboardTarget,
  TERMINAL_SHORTCUTS_ROOT_ATTR,
  withAppKeyboardTargetGuard,
} from './shortcutGuards';

interface MockTarget {
  tagName?: string;
  isContentEditable?: boolean;
  parentElement?: MockTarget | null;
  hasAttribute?: (name: string) => boolean;
}

function createTarget(
  options: {
    tagName?: string;
    isContentEditable?: boolean;
    parentElement?: MockTarget | null;
    attrs?: string[];
  } = {}
): MockTarget {
  const attrs = new Set(options.attrs ?? []);

  return {
    tagName: options.tagName,
    isContentEditable: options.isContentEditable ?? false,
    parentElement: options.parentElement ?? null,
    hasAttribute: (name: string) => attrs.has(name),
  };
}

describe('shortcutGuards', () => {
  it('ignores keyboard targets inside the embedded terminal boundary', () => {
    const terminalRoot = createTarget({
      attrs: [TERMINAL_SHORTCUTS_ROOT_ATTR],
    });
    const terminalChild = createTarget({ parentElement: terminalRoot });

    expect(
      shouldIgnoreAppKeyboardTarget(terminalChild as unknown as EventTarget)
    ).toBe(true);
  });

  it('still ignores standard editable targets', () => {
    const input = createTarget({ tagName: 'INPUT' });

    expect(shouldIgnoreAppKeyboardTarget(input as unknown as EventTarget)).toBe(
      true
    );
  });

  it('keeps shortcuts enabled for normal non-editable targets', () => {
    const button = createTarget({ tagName: 'BUTTON' });

    expect(
      shouldIgnoreAppKeyboardTarget(button as unknown as EventTarget)
    ).toBe(false);
  });

  it('disables app shortcuts when the active element is inside the terminal', () => {
    const terminalRoot = createTarget({
      attrs: [TERMINAL_SHORTCUTS_ROOT_ATTR],
    });
    const terminalCanvas = createTarget({ parentElement: terminalRoot });

    vi.stubGlobal('document', {
      activeElement: terminalCanvas,
    });

    expect(areAppKeyboardShortcutsEnabled()).toBe(false);

    vi.unstubAllGlobals();
  });

  it('disables guarded shortcuts when the live keyboard event target is inside the terminal', () => {
    const terminalRoot = createTarget({
      attrs: [TERMINAL_SHORTCUTS_ROOT_ATTR],
    });
    const terminalCanvas = createTarget({ parentElement: terminalRoot });
    const outsideButton = createTarget({ tagName: 'BUTTON' });

    vi.stubGlobal('document', {
      activeElement: outsideButton,
    });

    const guarded = withAppKeyboardTargetGuard(true) as (
      event?: KeyboardEvent
    ) => boolean;

    expect(
      guarded({ target: terminalCanvas } as unknown as KeyboardEvent)
    ).toBe(false);

    vi.unstubAllGlobals();
  });

  it('suppresses guarded shortcut callbacks when invoked from inside the terminal boundary', () => {
    const terminalRoot = createTarget({
      attrs: [TERMINAL_SHORTCUTS_ROOT_ATTR],
    });
    const terminalCanvas = createTarget({ parentElement: terminalRoot });
    const handler = vi.fn();

    const guarded = withAppKeyboardCallbackGuard(handler);

    guarded({ target: terminalCanvas } as unknown as KeyboardEvent);

    expect(handler).not.toHaveBeenCalled();
  });

  it('still runs guarded shortcut callbacks for non-terminal targets', () => {
    const button = createTarget({ tagName: 'BUTTON' });
    const handler = vi.fn();

    const guarded = withAppKeyboardCallbackGuard(handler);

    guarded({ target: button } as unknown as KeyboardEvent);

    expect(handler).toHaveBeenCalledOnce();
  });
});
