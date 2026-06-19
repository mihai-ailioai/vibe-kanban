import { mkdtemp, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { FileSessionStore, createEmptySessionState } from './state';

let stateDir: string;

beforeEach(async () => {
  stateDir = await mkdtemp(join(tmpdir(), 'vk-opencode-state-'));
});

afterEach(async () => {
  await rm(stateDir, { force: true, recursive: true });
});

describe('FileSessionStore', () => {
  it('keys state by OpenCode session id', async () => {
    const store = new FileSessionStore(stateDir);

    await store.save({
      ...createEmptySessionState('session-a'),
      binding: {
        origin: 'http://127.0.0.1:9000',
        projectId: 'project-a',
        issueId: 'issue-a',
      },
      trackingState: 'bound_idle',
    });

    expect(await store.load('session-b')).toEqual(
      createEmptySessionState('session-b')
    );
    expect((await store.load('session-a')).binding?.issueId).toBe('issue-a');
  });

  it('reloads persisted binding after process restart', async () => {
    await new FileSessionStore(stateDir).save({
      ...createEmptySessionState('session-a'),
      binding: {
        origin: 'https://vibe.example',
        projectId: 'project-a',
        issueId: 'issue-a',
      },
      trackingState: 'bound_idle',
    });

    expect(
      (await new FileSessionStore(stateDir).load('session-a')).binding
    ).toEqual({
      origin: 'https://vibe.example',
      projectId: 'project-a',
      issueId: 'issue-a',
    });
  });

  it('keeps pending entries across reloads', async () => {
    await new FileSessionStore(stateDir).save({
      ...createEmptySessionState('session-a'),
      pendingEntries: [
        {
          entry_id: 'entry-a',
          project_id: 'project-a',
          issue_id: 'issue-a',
          source_session_id: 'session-a',
          started_at: '2026-06-18T10:00:00.000Z',
          ended_at: '2026-06-18T10:01:00.000Z',
          duration_ms: 60000,
          metadata: { source: 'opencode' },
        },
      ],
    });

    expect(
      (await new FileSessionStore(stateDir).load('session-a')).pendingEntries
    ).toHaveLength(1);
  });
});
