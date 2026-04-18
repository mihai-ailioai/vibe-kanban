import { describe, expect, it } from 'vitest';
import { resolveExecutorConfigForSelection } from './executorSelection';

describe('resolveExecutorConfigForSelection', () => {
  it('keeps the saved default variant when reselecting the configured executor', () => {
    const config = resolveExecutorConfigForSelection({
      executor: 'CLAUDE_CODE',
      profiles: {
        CLAUDE_CODE: {
          DEFAULT: {},
          SONNET: {},
        },
        CODEX: {
          DEFAULT: {},
        },
      },
      configExecutorProfile: {
        executor: 'CLAUDE_CODE',
        variant: 'SONNET',
      },
    });

    expect(config).toEqual({
      executor: 'CLAUDE_CODE',
      variant: 'SONNET',
    });
  });

  it('falls back to DEFAULT when the configured variant is unavailable', () => {
    const config = resolveExecutorConfigForSelection({
      executor: 'CLAUDE_CODE',
      profiles: {
        CLAUDE_CODE: {
          DEFAULT: {},
          HAIKU: {},
        },
      },
      configExecutorProfile: {
        executor: 'CLAUDE_CODE',
        variant: 'SONNET',
      },
    });

    expect(config).toEqual({
      executor: 'CLAUDE_CODE',
      variant: 'DEFAULT',
    });
  });

  it('falls back to the first available variant when DEFAULT is missing', () => {
    const config = resolveExecutorConfigForSelection({
      executor: 'CLAUDE_CODE',
      profiles: {
        CLAUDE_CODE: {
          OPUS: {},
          SONNET: {},
        },
      },
      configExecutorProfile: null,
    });

    expect(config).toEqual({
      executor: 'CLAUDE_CODE',
      variant: 'OPUS',
    });
  });
});
