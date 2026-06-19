import { mkdir, readFile, rename, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

export type TrackingState =
  | 'unbound'
  | 'bound_idle'
  | 'bound_active'
  | 'bound_waiting';

export interface IssueBinding {
  origin: string;
  projectId: string;
  issueId: string;
}

export interface ActiveInterval {
  startedAtMs: number;
}

export interface PendingEntry {
  entry_id: string;
  project_id: string;
  issue_id: string;
  source_session_id: string | null;
  started_at: string;
  ended_at: string;
  duration_ms: number;
  metadata: Record<string, unknown>;
}

export interface SessionState {
  sessionId: string;
  trackingState: TrackingState;
  binding: IssueBinding | null;
  activeInterval: ActiveInterval | null;
  pendingEntries: PendingEntry[];
}

export function createEmptySessionState(sessionId: string): SessionState {
  return {
    sessionId,
    trackingState: 'unbound',
    binding: null,
    activeInterval: null,
    pendingEntries: [],
  };
}

export class FileSessionStore {
  constructor(private readonly stateDir: string) {}

  async load(sessionId: string): Promise<SessionState> {
    try {
      const raw = await readFile(this.pathForSession(sessionId), 'utf8');
      return normalizeState(sessionId, JSON.parse(raw));
    } catch (error) {
      if (isNotFoundError(error)) {
        return createEmptySessionState(sessionId);
      }
      throw error;
    }
  }

  async save(state: SessionState): Promise<void> {
    await mkdir(this.stateDir, { recursive: true });
    const path = this.pathForSession(state.sessionId);
    const tmpPath = `${path}.${process.pid}.${Date.now()}.tmp`;
    await writeFile(tmpPath, `${JSON.stringify(state, null, 2)}\n`, 'utf8');
    await rename(tmpPath, path);
  }

  private pathForSession(sessionId: string): string {
    return join(this.stateDir, `${encodeURIComponent(sessionId)}.json`);
  }
}

function normalizeState(sessionId: string, value: unknown): SessionState {
  if (!value || typeof value !== 'object') {
    return createEmptySessionState(sessionId);
  }
  const candidate = value as Partial<SessionState>;
  return {
    sessionId,
    trackingState: candidate.trackingState ?? 'unbound',
    binding: candidate.binding ?? null,
    activeInterval: candidate.activeInterval ?? null,
    pendingEntries: candidate.pendingEntries ?? [],
  };
}

function isNotFoundError(error: unknown): boolean {
  return (
    typeof error === 'object' &&
    error !== null &&
    'code' in error &&
    error.code === 'ENOENT'
  );
}
