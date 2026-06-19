import { mkdtemp, readFile, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import plugin from './index';

const projectId = '11111111-1111-4111-8111-111111111111';
const issueId = '22222222-2222-4222-8222-222222222222';

let stateDir: string;

beforeEach(async () => {
  stateDir = await mkdtemp(join(tmpdir(), 'vk-opencode-plugin-'));
  process.env.VIBE_KANBAN_OPENCODE_TIME_TRACKER_STATE_DIR = stateDir;
});

afterEach(async () => {
  delete process.env.VIBE_KANBAN_OPENCODE_TIME_TRACKER_STATE_DIR;
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  await rm(stateDir, { force: true, recursive: true });
});

describe('OpenCode plugin entrypoint', () => {
  it('returns defensive hooks without editing OpenCode config', async () => {
    const hooks = await plugin(
      { directory: '/tmp/project' },
      { servers: { 'http://127.0.0.1:9000': { token: 'vktt_secret' } } }
    );

    expect(hooks).toMatchObject({});
    expect(typeof hooks.event).toBe('function');
    expect(typeof hooks['chat.message']).toBe('function');
  });

  it('no-ops chat and tool hooks without a session id', async () => {
    const hooks = await plugin(
      { directory: '/tmp/project' },
      { servers: { 'http://127.0.0.1:9000': { token: 'vktt_secret' } } }
    );

    await expect(
      hooks['chat.message']?.(
        {},
        {
          message: {
            text: `Track http://127.0.0.1:9000/projects/${projectId}/issues/${issueId}`,
          },
          parts: [],
        }
      )
    ).resolves.toBeUndefined();
    await expect(
      hooks['tool.execute.before']?.(
        { tool: 'bash', callID: 'call-a' },
        { args: {} }
      )
    ).resolves.toBeUndefined();
    await expect(
      hooks['tool.execute.after']?.(
        { tool: 'bash', callID: 'call-a' },
        { title: 'done', output: '', metadata: {} }
      )
    ).resolves.toBeUndefined();

    await expect(readdir(stateDir)).resolves.not.toContain('undefined.json');
    await expect(readdir(stateDir)).resolves.toEqual([]);
  });

  it('keeps pending entries when options omit servers', async () => {
    const hooks = await plugin({ directory: '/tmp/project' }, {} as never);

    await hooks['chat.message']?.(
      { sessionID: 'session-a' },
      {
        message: {
          text: `Track http://127.0.0.1:9000/projects/${projectId}/issues/${issueId}`,
        },
        parts: [],
      }
    );
    await expect(
      hooks.event?.({
        event: { type: 'session.idle', sessionID: 'session-a' },
      })
    ).resolves.toBeUndefined();

    const persisted = JSON.parse(
      await readFile(join(stateDir, 'session-a.json'), 'utf8')
    );
    expect(persisted.pendingEntries).toHaveLength(1);
  });

  it('keeps pending entries when successful server responses are malformed', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({
            success: true,
            data: { txid: 'bad', results: [], updated_totals: [] },
            error_data: null,
            message: null,
          })
        )
      )
    );
    const hooks = await plugin(
      { directory: '/tmp/project' },
      { servers: { 'http://127.0.0.1:9000': { token: 'vktt_secret' } } }
    );

    await hooks['chat.message']?.(
      { sessionID: 'session-a' },
      {
        message: {
          text: `Track http://127.0.0.1:9000/projects/${projectId}/issues/${issueId}`,
        },
        parts: [],
      }
    );
    await hooks.event?.({
      event: { type: 'session.idle', sessionID: 'session-a' },
    });

    const persisted = JSON.parse(
      await readFile(join(stateDir, 'session-a.json'), 'utf8')
    );
    expect(persisted.pendingEntries).toHaveLength(1);
  });

  it('serializes same-session updates without write races', async () => {
    const hooks = await plugin({ directory: '/tmp/project' }, {} as never);

    await hooks['chat.message']?.(
      { sessionID: 'session-a' },
      {
        message: {
          text: `Track http://127.0.0.1:9000/projects/${projectId}/issues/${issueId}`,
        },
        parts: [],
      }
    );

    await Promise.all([
      hooks.event?.({
        event: { type: 'session.idle', sessionID: 'session-a' },
      }),
      hooks['chat.message']?.(
        { sessionID: 'session-a' },
        {
          message: {
            text: `Switch http://127.0.0.1:9000/projects/${projectId}/issues/33333333-3333-4333-8333-333333333333`,
          },
          parts: [],
        }
      ),
    ]);
    await hooks.event?.({
      event: { type: 'session.idle', sessionID: 'session-a' },
    });

    const persisted = JSON.parse(
      await readFile(join(stateDir, 'session-a.json'), 'utf8')
    );
    expect(persisted.sessionId).toBe('session-a');
    expect(persisted.pendingEntries.length).toBeGreaterThanOrEqual(1);
  });

  it('caps intervals on ticket switch and ignores invalid cap options', async () => {
    let now = 0;
    vi.spyOn(Date, 'now').mockImplementation(() => now);
    const nextIssueId = '33333333-3333-4333-8333-333333333333';
    const cappedHooks = await plugin(
      { directory: '/tmp/project' },
      { servers: {}, maxRecoveredIntervalMs: 30000 }
    );

    await cappedHooks['chat.message']?.(
      { sessionID: 'capped-session' },
      {
        message: {
          text: `Track http://127.0.0.1:9000/projects/${projectId}/issues/${issueId}`,
        },
        parts: [],
      }
    );
    now = 120000;
    await cappedHooks['chat.message']?.(
      { sessionID: 'capped-session' },
      {
        message: {
          text: `Switch http://127.0.0.1:9000/projects/${projectId}/issues/${nextIssueId}`,
        },
        parts: [],
      }
    );

    const capped = JSON.parse(
      await readFile(join(stateDir, 'capped-session.json'), 'utf8')
    );
    expect(capped.pendingEntries[0].duration_ms).toBe(30000);

    const invalidHooks = await plugin(
      { directory: '/tmp/project' },
      { servers: {}, maxRecoveredIntervalMs: -1 }
    );
    now = 0;
    await invalidHooks['chat.message']?.(
      { sessionID: 'invalid-cap-session' },
      {
        message: {
          text: `Track http://127.0.0.1:9000/projects/${projectId}/issues/${issueId}`,
        },
        parts: [],
      }
    );
    now = 120000;
    await invalidHooks.event?.({
      event: { type: 'session.idle', sessionID: 'invalid-cap-session' },
    });

    const invalid = JSON.parse(
      await readFile(join(stateDir, 'invalid-cap-session.json'), 'utf8')
    );
    expect(invalid.pendingEntries[0].duration_ms).toBe(120000);
  });

  it('ignores malformed hook payloads and malformed server options', async () => {
    const hooks = await plugin({ directory: '/tmp/project' }, {
      servers: { 'http://127.0.0.1:9000': { token: 123 } },
    } as never);

    await expect(hooks.event?.(null as never)).resolves.toBeUndefined();
    await expect(
      hooks['chat.message']?.({ sessionID: 'session-a' }, null as never)
    ).resolves.toBeUndefined();
    await expect(
      hooks['chat.message']?.(
        { sessionID: 'session-a' },
        {
          message: {
            text: `Track http://127.0.0.1:9000/projects/${projectId}/issues/${issueId}`,
          },
          parts: [],
        }
      )
    ).resolves.toBeUndefined();
    await expect(
      hooks.event?.({ event: { type: 'session.idle', sessionID: 'session-a' } })
    ).resolves.toBeUndefined();

    const persisted = JSON.parse(
      await readFile(join(stateDir, 'session-a.json'), 'utf8')
    );
    expect(persisted.pendingEntries).toHaveLength(1);
  });
});
