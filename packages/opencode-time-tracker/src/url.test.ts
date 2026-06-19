import { describe, expect, it } from 'vitest';

import { findVibeIssueUrls, parseVibeIssueUrl } from './url';

const projectId = '11111111-1111-4111-8111-111111111111';
const issueId = '22222222-2222-4222-8222-222222222222';

describe('vibe-kanban issue URL parsing', () => {
  it('parses localhost vibe issue URLs', () => {
    expect(
      parseVibeIssueUrl(
        `http://127.0.0.1:9000/projects/${projectId}/issues/${issueId}`
      )
    ).toEqual({
      origin: 'http://127.0.0.1:9000',
      projectId,
      issueId,
      url: `http://127.0.0.1:9000/projects/${projectId}/issues/${issueId}`,
    });
  });

  it('parses https host vibe issue URLs', () => {
    expect(
      parseVibeIssueUrl(
        `https://vibe.example/projects/${projectId}/issues/${issueId}`
      )
    ).toMatchObject({
      origin: 'https://vibe.example',
      projectId,
      issueId,
    });
  });

  it('ignores non-vibe and malformed URLs', () => {
    expect(parseVibeIssueUrl('https://example.com/not-vibe')).toBeNull();
    expect(
      parseVibeIssueUrl(
        `https://vibe.example/projects/${projectId}/issues/not-a-uuid`
      )
    ).toBeNull();
  });

  it('returns all valid URLs in order so the latest can win', () => {
    const nextIssueId = '33333333-3333-4333-8333-333333333333';

    expect(
      findVibeIssueUrls(
        `First https://vibe.example/projects/${projectId}/issues/${issueId} then https://vibe.example/projects/${projectId}/issues/${nextIssueId}.`
      ).map((issue) => issue.issueId)
    ).toEqual([issueId, nextIssueId]);
  });
});
