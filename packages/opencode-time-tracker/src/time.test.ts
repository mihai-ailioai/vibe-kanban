import { describe, expect, it } from 'vitest';

import {
  bindIssue,
  closeActiveInterval,
  createEmptySessionState,
  enterWaiting,
  resumeActive,
  startActive,
} from './time';

const binding = {
  origin: 'http://127.0.0.1:9000',
  projectId: 'project-a',
  issueId: 'issue-a',
};

const uuidPattern =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

describe('active time state machine', () => {
  it('creates no entries for unbound sessions', () => {
    const state = startActive(createEmptySessionState('session-a'), 1000);

    expect(closeActiveInterval(state, 2000).pendingEntries).toEqual([]);
  });

  it('closes a bound active interval on idle with UUID entry id', () => {
    const state = closeActiveInterval(
      startActive(
        bindIssue(createEmptySessionState('session-a'), binding, 1000),
        1000
      ),
      61000
    );

    expect(state.trackingState).toBe('bound_idle');
    expect(state.pendingEntries[0]).toMatchObject({
      project_id: 'project-a',
      issue_id: 'issue-a',
      duration_ms: 60000,
      metadata: { source: 'opencode', origin: 'http://127.0.0.1:9000' },
    });
    expect(state.pendingEntries[0].entry_id).toMatch(uuidPattern);
  });

  it('waiting closes the active interval and resume starts a later one', () => {
    const waiting = enterWaiting(
      startActive(
        bindIssue(createEmptySessionState('session-a'), binding, 1000),
        1000
      ),
      11000
    );
    const resumed = resumeActive(waiting, 21000);
    const idle = closeActiveInterval(resumed, 31000);

    expect(waiting.trackingState).toBe('bound_waiting');
    expect(idle.pendingEntries.map((entry) => entry.duration_ms)).toEqual([
      10000, 10000,
    ]);
  });

  it('caps suspicious recovered intervals', () => {
    const state = closeActiveInterval(
      startActive(
        bindIssue(createEmptySessionState('session-a'), binding, 0),
        0
      ),
      120000,
      { maxRecoveredIntervalMs: 30000 }
    );

    expect(state.pendingEntries[0].duration_ms).toBe(30000);
    expect(state.pendingEntries[0].ended_at).toBe('1970-01-01T00:00:30.000Z');
  });

  it('ticket switches affect future entries only', () => {
    const next = { ...binding, issueId: 'issue-b' };
    const switched = bindIssue(
      startActive(
        bindIssue(createEmptySessionState('session-a'), binding, 1000),
        1000
      ),
      next,
      11000
    );
    const done = closeActiveInterval(startActive(switched, 12000), 22000);

    expect(done.pendingEntries.map((entry) => entry.issue_id)).toEqual([
      'issue-a',
      'issue-b',
    ]);
  });

  it('caps active intervals closed by ticket switches', () => {
    const next = { ...binding, issueId: 'issue-b' };
    const switched = bindIssue(
      startActive(
        bindIssue(createEmptySessionState('session-a'), binding, 0),
        0
      ),
      next,
      120000,
      { maxRecoveredIntervalMs: 30000 }
    );

    expect(switched.pendingEntries[0]).toMatchObject({
      issue_id: 'issue-a',
      duration_ms: 30000,
      ended_at: '1970-01-01T00:00:30.000Z',
    });
  });
});
