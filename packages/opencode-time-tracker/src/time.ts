import { randomUUID } from 'node:crypto';

import {
  type IssueBinding,
  type PendingEntry,
  type SessionState,
  createEmptySessionState,
} from './state';

export { createEmptySessionState } from './state';

export interface CloseIntervalOptions {
  maxRecoveredIntervalMs?: number;
}

export function bindIssue(
  state: SessionState,
  binding: IssueBinding,
  nowMs: number,
  options: CloseIntervalOptions = {}
): SessionState {
  const closed = closeActiveInterval(state, nowMs, options);
  return {
    ...closed,
    binding,
    activeInterval: null,
    trackingState: 'bound_idle',
  };
}

export function startActive(state: SessionState, nowMs: number): SessionState {
  if (!state.binding) {
    return state;
  }
  if (state.trackingState === 'bound_active') {
    return state;
  }
  return {
    ...state,
    trackingState: 'bound_active',
    activeInterval: { startedAtMs: nowMs },
  };
}

export function enterWaiting(state: SessionState, nowMs: number): SessionState {
  if (!state.binding) {
    return state;
  }
  const closed = closeActiveInterval(state, nowMs);
  return {
    ...closed,
    trackingState: 'bound_waiting',
    activeInterval: null,
  };
}

export function resumeActive(state: SessionState, nowMs: number): SessionState {
  return startActive(state, nowMs);
}

export function closeActiveInterval(
  state: SessionState,
  nowMs: number,
  options: CloseIntervalOptions = {}
): SessionState {
  if (!state.binding || !state.activeInterval) {
    return state.binding ? { ...state, trackingState: 'bound_idle' } : state;
  }

  const cappedDurationMs = capDuration(
    nowMs - state.activeInterval.startedAtMs,
    options.maxRecoveredIntervalMs
  );
  if (cappedDurationMs <= 0) {
    return { ...state, trackingState: 'bound_idle', activeInterval: null };
  }

  const entry = createEntry(
    state.sessionId,
    state.binding,
    state.activeInterval.startedAtMs,
    cappedDurationMs
  );

  return {
    ...state,
    trackingState: 'bound_idle',
    activeInterval: null,
    pendingEntries: [...state.pendingEntries, entry],
  };
}

function capDuration(
  durationMs: number,
  maxRecoveredIntervalMs?: number
): number {
  if (maxRecoveredIntervalMs === undefined) {
    return durationMs;
  }
  return Math.min(durationMs, maxRecoveredIntervalMs);
}

function createEntry(
  sessionId: string,
  binding: IssueBinding,
  startedAtMs: number,
  durationMs: number
): PendingEntry {
  const endedAtMs = startedAtMs + durationMs;
  return {
    entry_id: randomUUID(),
    project_id: binding.projectId,
    issue_id: binding.issueId,
    source_session_id: sessionId,
    started_at: new Date(startedAtMs).toISOString(),
    ended_at: new Date(endedAtMs).toISOString(),
    duration_ms: durationMs,
    metadata: { source: 'opencode', origin: binding.origin },
  };
}

export function emptyStateFor(sessionId: string): SessionState {
  return createEmptySessionState(sessionId);
}
