import { beforeEach, describe, expect, it } from 'vitest';
import {
  DEFAULT_HIDE_THINKING_MESSAGES,
  useUiPreferencesStore,
} from './useUiPreferencesStore';

describe('useUiPreferencesStore', () => {
  beforeEach(() => {
    useUiPreferencesStore.setState(useUiPreferencesStore.getInitialState());
  });

  it('defaults hideThinkingMessages to true', () => {
    expect(DEFAULT_HIDE_THINKING_MESSAGES).toBe(true);
    expect(useUiPreferencesStore.getState().hideThinkingMessages).toBe(true);
  });

  it('updates hideThinkingMessages through the setter', () => {
    useUiPreferencesStore.getState().setHideThinkingMessages(false);

    expect(useUiPreferencesStore.getState().hideThinkingMessages).toBe(false);
  });
});
