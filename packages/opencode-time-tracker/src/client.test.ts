import { afterEach, describe, expect, it, vi } from 'vitest';

import { postEntries } from './client';
import type { PendingEntry } from './state';

const entry: PendingEntry = {
  entry_id: 'entry-a',
  project_id: 'project-a',
  issue_id: 'issue-a',
  source_session_id: 'session-a',
  started_at: '2026-06-18T10:00:00.000Z',
  ended_at: '2026-06-18T10:01:00.000Z',
  duration_ms: 60000,
  metadata: { source: 'opencode' },
};

afterEach(() => {
  vi.restoreAllMocks();
});

describe('postEntries', () => {
  it('posts entries to the local endpoint with bearer token', async () => {
    const fetch = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          success: true,
          data: { txid: 1, results: [], updated_totals: [] },
          error_data: null,
          message: null,
        }),
        { status: 200 }
      )
    );

    await postEntries('http://127.0.0.1:9000', 'vktt_secret', [entry], {
      fetch,
    });

    expect(fetch).toHaveBeenCalledWith(
      'http://127.0.0.1:9000/api/time-tracking/opencode/entries',
      expect.objectContaining({
        method: 'POST',
        headers: expect.objectContaining({
          Authorization: 'Bearer vktt_secret',
        }),
        body: JSON.stringify({ schema_version: 1, entries: [entry] }),
      })
    );
  });

  it('unwraps valid local ApiResponse envelopes', async () => {
    const localFetch = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          success: true,
          data: { txid: 1, results: [], updated_totals: [] },
          error_data: null,
          message: null,
        })
      )
    );

    await expect(
      postEntries('http://127.0.0.1:9000', 'vktt_secret', [entry], {
        fetch: localFetch,
      })
    ).resolves.toMatchObject({ txid: 1 });
  });

  it('rejects bare and malformed successful responses', async () => {
    const bareFetch = vi
      .fn()
      .mockResolvedValue(
        new Response(
          JSON.stringify({ txid: 2, results: [], updated_totals: [] })
        )
      );
    const malformedFetch = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          success: true,
          data: { txid: 'not-a-number', results: [], updated_totals: [] },
          error_data: null,
          message: null,
        })
      )
    );

    await expect(
      postEntries('http://127.0.0.1:9000', 'vktt_secret', [entry], {
        fetch: bareFetch,
      })
    ).rejects.toThrow('Malformed OpenCode time entries response');
    await expect(
      postEntries('http://127.0.0.1:9000', 'vktt_secret', [entry], {
        fetch: malformedFetch,
      })
    ).rejects.toThrow('Malformed OpenCode time entries response');
  });

  it('rejects failures so callers retain pending entries', async () => {
    const fetch = vi
      .fn()
      .mockResolvedValue(
        new Response(JSON.stringify({ message: 'nope' }), { status: 500 })
      );

    await expect(
      postEntries('http://127.0.0.1:9000', 'vktt_secret', [entry], { fetch })
    ).rejects.toThrow('Failed to post OpenCode time entries');
  });
});
