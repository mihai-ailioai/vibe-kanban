import { homedir } from 'node:os';
import { join } from 'node:path';

import type { Hooks, PluginInput } from '@opencode-ai/plugin';

import { postEntries } from './client';
import {
  FileSessionStore,
  type PendingEntry,
  type SessionState,
} from './state';
import {
  bindIssue,
  closeActiveInterval,
  enterWaiting,
  startActive,
} from './time';
import { findVibeIssueUrls } from './url';

export interface VibeKanbanTimeTrackerOptions {
  servers: Record<string, { token: string }>;
  maxRecoveredIntervalMs?: number;
}

type Plugin = (
  input: Partial<PluginInput>,
  options?: Partial<VibeKanbanTimeTrackerOptions> | null
) => Promise<Hooks>;

interface NormalizedOptions {
  servers: Record<string, { token: string }>;
  maxRecoveredIntervalMs?: number;
}

const plugin: Plugin = async (_input, options = {}) => {
  const store = new FileSessionStore(resolveStateDir());
  const normalizedOptions = normalizeOptions(options);
  const sessionQueues = new Map<string, Promise<void>>();

  async function enqueueSessionUpdate(
    sessionId: string,
    operation: () => Promise<void>
  ): Promise<void> {
    const previous = sessionQueues.get(sessionId) ?? Promise.resolve();
    const next = previous.catch(() => undefined).then(operation);
    const tracked = next.catch(() => undefined);
    sessionQueues.set(sessionId, tracked);
    try {
      await next;
    } finally {
      if (sessionQueues.get(sessionId) === tracked) {
        sessionQueues.delete(sessionId);
      }
    }
  }

  async function updateSession(
    sessionId: string,
    update: (
      state: SessionState,
      nowMs: number
    ) => SessionState | Promise<SessionState>
  ): Promise<void> {
    await enqueueSessionUpdate(sessionId, async () => {
      const state = await store.load(sessionId);
      const next = await update(state, Date.now());
      const flushed = await flushPending(next, normalizedOptions);
      await store.save(flushed);
    });
  }

  return {
    'chat.message': async (input: unknown, output: unknown) => {
      const sessionId = extractSessionId(input);
      if (!sessionId) {
        return;
      }
      const text = extractText(output);
      const issueUrls = findVibeIssueUrls(text);
      await updateSession(sessionId, (state, nowMs) => {
        const latestIssueUrl = issueUrls.at(-1);
        const bound = latestIssueUrl
          ? bindIssue(
              state,
              {
                origin: latestIssueUrl.origin,
                projectId: latestIssueUrl.projectId,
                issueId: latestIssueUrl.issueId,
              },
              nowMs,
              {
                maxRecoveredIntervalMs:
                  normalizedOptions.maxRecoveredIntervalMs,
              }
            )
          : state;
        return startActive(bound, nowMs);
      });
    },
    event: async (input: unknown) => {
      const event = getRecordValue(input, 'event');
      const sessionId = extractSessionId(event);
      if (!sessionId) {
        return;
      }
      const eventName = extractEventName(event);
      if (eventName === 'session.idle' || eventName === 'session.deleted') {
        await updateSession(sessionId, (state, nowMs) =>
          closeActiveInterval(state, nowMs, {
            maxRecoveredIntervalMs: normalizedOptions.maxRecoveredIntervalMs,
          })
        );
      }
      if (eventName === 'session.status') {
        const status = extractStatus(event);
        if (status === 'idle') {
          await updateSession(sessionId, (state, nowMs) =>
            closeActiveInterval(state, nowMs, {
              maxRecoveredIntervalMs: normalizedOptions.maxRecoveredIntervalMs,
            })
          );
        }
        if (status === 'waiting') {
          await updateSession(sessionId, (state, nowMs) =>
            enterWaiting(state, nowMs)
          );
        }
      }
    },
    'permission.ask': async (input: unknown) => {
      const sessionId = extractSessionId(input);
      if (sessionId) {
        await updateSession(sessionId, (state, nowMs) =>
          enterWaiting(state, nowMs)
        );
      }
    },
    'tool.execute.before': async (input: unknown) => {
      const sessionId = extractSessionId(input);
      if (!sessionId) {
        return;
      }
      await updateSession(sessionId, (state, nowMs) =>
        startActive(state, nowMs)
      );
    },
    'tool.execute.after': async (input: unknown) => {
      const sessionId = extractSessionId(input);
      if (!sessionId) {
        return;
      }
      await updateSession(sessionId, (state, nowMs) =>
        startActive(state, nowMs)
      );
    },
  };
};

export default plugin;

function normalizeOptions(
  options: Partial<VibeKanbanTimeTrackerOptions> | null | undefined
): NormalizedOptions {
  return {
    servers: normalizeServers(options?.servers),
    maxRecoveredIntervalMs: normalizeMaxRecoveredIntervalMs(
      options?.maxRecoveredIntervalMs
    ),
  };
}

function normalizeServers(value: unknown): Record<string, { token: string }> {
  if (typeof value !== 'object' || value === null) {
    return {};
  }
  const servers: Record<string, { token: string }> = {};
  for (const [origin, config] of Object.entries(value)) {
    if (typeof config !== 'object' || config === null) {
      continue;
    }
    const token = (config as Record<string, unknown>).token;
    if (typeof token === 'string') {
      servers[origin] = { token };
    }
  }
  return servers;
}

function normalizeMaxRecoveredIntervalMs(value: unknown): number | undefined {
  if (typeof value !== 'number' || !Number.isFinite(value) || value <= 0) {
    return undefined;
  }
  return value;
}

async function flushPending(
  state: SessionState,
  options: NormalizedOptions
): Promise<SessionState> {
  if (state.pendingEntries.length === 0) {
    return state;
  }

  const pendingByOrigin = groupPendingByOrigin(state);
  const sentEntryIds = new Set<string>();

  for (const [origin, entries] of pendingByOrigin) {
    const token = options.servers[origin]?.token;
    if (!token) {
      continue;
    }
    try {
      await postEntries(origin, token, entries);
      entries.forEach((entry) => sentEntryIds.add(entry.entry_id));
    } catch {
      // Keep entries pending. The next hook invocation will retry.
    }
  }

  if (sentEntryIds.size === 0) {
    return state;
  }

  return {
    ...state,
    pendingEntries: state.pendingEntries.filter(
      (entry) => !sentEntryIds.has(entry.entry_id)
    ),
  };
}

function groupPendingByOrigin(
  state: SessionState
): Map<string, PendingEntry[]> {
  const entriesByOrigin = new Map<string, PendingEntry[]>();
  for (const entry of state.pendingEntries) {
    const origin =
      typeof entry.metadata.origin === 'string'
        ? entry.metadata.origin
        : state.binding?.origin;
    if (!origin) {
      continue;
    }
    entriesByOrigin.set(origin, [
      ...(entriesByOrigin.get(origin) ?? []),
      entry,
    ]);
  }
  return entriesByOrigin;
}

function extractText(output: unknown): string {
  const fragments: string[] = [];
  collectText(getRecordValue(output, 'message'), fragments);
  collectText(getRecordValue(output, 'parts'), fragments);
  return fragments.join('\n');
}

function collectText(value: unknown, fragments: string[]): void {
  if (typeof value === 'string') {
    fragments.push(value);
    return;
  }
  if (Array.isArray(value)) {
    value.forEach((item) => collectText(item, fragments));
    return;
  }
  if (typeof value !== 'object' || value === null) {
    return;
  }
  const record = value as Record<string, unknown>;
  collectText(record.text, fragments);
  collectText(record.content, fragments);
}

function extractEventName(event: unknown): string | null {
  if (typeof event !== 'object' || event === null) {
    return null;
  }
  const record = event as Record<string, unknown>;
  return stringValue(record.type) ?? stringValue(record.name);
}

function extractSessionId(value: unknown): string | null {
  if (typeof value !== 'object' || value === null) {
    return null;
  }
  const record = value as Record<string, unknown>;
  return (
    stringValue(record.sessionID) ??
    stringValue(record.sessionId) ??
    stringValue(record.session_id) ??
    extractSessionId(record.session)
  );
}

function extractStatus(event: unknown): string | null {
  if (typeof event !== 'object' || event === null) {
    return null;
  }
  const record = event as Record<string, unknown>;
  return stringValue(record.status) ?? stringValue(record.state);
}

function getRecordValue(value: unknown, key: string): unknown {
  if (typeof value !== 'object' || value === null) {
    return undefined;
  }
  return (value as Record<string, unknown>)[key];
}

function stringValue(value: unknown): string | null {
  return typeof value === 'string' ? value : null;
}

function resolveStateDir(): string {
  return (
    process.env.VIBE_KANBAN_OPENCODE_TIME_TRACKER_STATE_DIR ??
    process.env.OPENCODE_STATE_DIR ??
    join(homedir(), '.local', 'state', 'opencode', 'vibe-kanban-time-tracker')
  );
}
