import type { PendingEntry } from './state';

export interface OpenCodeTimeEntryResult {
  entry_id: string;
  status: 'created' | 'duplicate';
  project_id: string;
  issue_id: string;
  duration_ms: number;
}

export interface CreateOpenCodeTimeEntriesResponse {
  txid: number;
  results: OpenCodeTimeEntryResult[];
  updated_totals: unknown[];
}

export interface PostEntriesOptions {
  fetch?: typeof fetch;
}

export async function postEntries(
  origin: string,
  token: string,
  entries: PendingEntry[],
  options: PostEntriesOptions = {}
): Promise<CreateOpenCodeTimeEntriesResponse> {
  const fetchImpl = options.fetch ?? fetch;
  const response = await fetchImpl(
    `${origin}/api/time-tracking/opencode/entries`,
    {
      method: 'POST',
      headers: {
        Authorization: `Bearer ${token}`,
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ schema_version: 1, entries }),
    }
  );

  const json = await parseJson(response);
  if (!response.ok) {
    throw new Error(
      `Failed to post OpenCode time entries: ${response.status} ${messageFrom(json)}`.trim()
    );
  }

  return unwrapResponse(json);
}

async function parseJson(response: Response): Promise<unknown> {
  const text = await response.text();
  if (text.length === 0) {
    return null;
  }
  return JSON.parse(text);
}

function unwrapResponse(value: unknown): CreateOpenCodeTimeEntriesResponse {
  if (!isApiResponse(value)) {
    throw new Error('Malformed OpenCode time entries response');
  }
  if (!value.success || value.data === null) {
    throw new Error(
      `Failed to post OpenCode time entries: ${messageFrom(value)}`.trim()
    );
  }
  if (!isCreateOpenCodeTimeEntriesResponse(value.data)) {
    throw new Error('Malformed OpenCode time entries response');
  }
  return value.data;
}

function isApiResponse(value: unknown): value is {
  success: boolean;
  data: unknown;
  message: string | null;
} {
  return (
    typeof value === 'object' &&
    value !== null &&
    'success' in value &&
    typeof value.success === 'boolean' &&
    'data' in value
  );
}

function messageFrom(value: unknown): string {
  if (typeof value === 'object' && value !== null && 'message' in value) {
    return typeof value.message === 'string' ? value.message : '';
  }
  return '';
}

function isCreateOpenCodeTimeEntriesResponse(
  value: unknown
): value is CreateOpenCodeTimeEntriesResponse {
  if (typeof value !== 'object' || value === null) {
    return false;
  }
  const record = value as Record<string, unknown>;
  return (
    typeof record.txid === 'number' &&
    Number.isFinite(record.txid) &&
    Array.isArray(record.results) &&
    record.results.every(isOpenCodeTimeEntryResult) &&
    Array.isArray(record.updated_totals)
  );
}

function isOpenCodeTimeEntryResult(
  value: unknown
): value is OpenCodeTimeEntryResult {
  if (typeof value !== 'object' || value === null) {
    return false;
  }
  const record = value as Record<string, unknown>;
  return (
    typeof record.entry_id === 'string' &&
    (record.status === 'created' || record.status === 'duplicate') &&
    typeof record.project_id === 'string' &&
    typeof record.issue_id === 'string' &&
    typeof record.duration_ms === 'number' &&
    Number.isFinite(record.duration_ms)
  );
}
