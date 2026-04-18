import { describe, expect, it } from 'vitest';
import type { PatchTypeWithKey } from '@/shared/hooks/useConversationHistory/types';

import { deriveConversationTimeline } from './deriveConversationTimeline';

function createNormalizedEntry(
  patchKey: string,
  entryType: PatchTypeWithKey['content']['entry_type']
): PatchTypeWithKey {
  return {
    type: 'NORMALIZED_ENTRY',
    patchKey,
    executionProcessId: 'process-1',
    content: {
      timestamp: null,
      entry_type: entryType,
      content: patchKey,
    },
  };
}

describe('deriveConversationTimeline', () => {
  const entries: PatchTypeWithKey[] = [
    createNormalizedEntry('user-1', { type: 'user_message' }),
    createNormalizedEntry('thinking-previous', { type: 'thinking' }),
    createNormalizedEntry('assistant-1', { type: 'assistant_message' }),
    createNormalizedEntry('user-2', { type: 'user_message' }),
    createNormalizedEntry('thinking-live', { type: 'thinking' }),
    createNormalizedEntry('assistant-2', { type: 'assistant_message' }),
  ];

  it('filters live and aggregated thinking entries when hideThinkingMessages is true', () => {
    const timeline = deriveConversationTimeline(entries, [], [], {
      hideThinkingMessages: true,
    });

    expect(timeline.displayEntries.map((entry) => entry.patchKey)).toEqual([
      'user-1',
      'assistant-1',
      'user-2',
      'assistant-2',
    ]);
    expect(
      timeline.displayEntries.some(
        (entry) => entry.type === 'AGGREGATED_THINKING_GROUP'
      )
    ).toBe(false);
  });

  it('keeps thinking entries visible when hideThinkingMessages is false', () => {
    const timeline = deriveConversationTimeline(entries, [], [], {
      hideThinkingMessages: false,
    });

    expect(timeline.displayEntries.map((entry) => entry.patchKey)).toEqual([
      'user-1',
      'agg-thinking:thinking-previous',
      'assistant-1',
      'user-2',
      'thinking-live',
      'assistant-2',
    ]);
    expect(
      timeline.displayEntries.some(
        (entry) => entry.type === 'AGGREGATED_THINKING_GROUP'
      )
    ).toBe(true);
  });

  it('re-derives already-built chat when hideThinkingMessages toggles on', () => {
    const visibleTimeline = deriveConversationTimeline(entries, [], [], {
      hideThinkingMessages: false,
    });

    const hiddenTimeline = deriveConversationTimeline(
      entries,
      visibleTimeline.displayEntries,
      visibleTimeline.rows,
      {
        hideThinkingMessages: true,
      }
    );

    expect(
      hiddenTimeline.displayEntries.map((entry) => entry.patchKey)
    ).toEqual(['user-1', 'assistant-1', 'user-2', 'assistant-2']);
    expect(
      hiddenTimeline.displayEntries.some(
        (entry) => entry.type === 'AGGREGATED_THINKING_GROUP'
      )
    ).toBe(false);
    expect(
      hiddenTimeline.displayEntries.some(
        (entry) =>
          entry.type === 'NORMALIZED_ENTRY' &&
          entry.content.entry_type.type === 'thinking'
      )
    ).toBe(false);
  });
});
