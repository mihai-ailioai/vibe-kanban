export interface VibeIssueUrl {
  origin: string;
  projectId: string;
  issueId: string;
  url: string;
}

const UUID_RE =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const URL_RE = /https?:\/\/[^\s<>()]+/gi;

export function parseVibeIssueUrl(value: string): VibeIssueUrl | null {
  const candidate = trimTrailingPunctuation(value);

  let url: URL;
  try {
    url = new URL(candidate);
  } catch {
    return null;
  }

  const segments = url.pathname.split('/').filter(Boolean);
  if (segments.length !== 4) {
    return null;
  }

  const [projects, projectId, issues, issueId] = segments;
  if (projects !== 'projects' || issues !== 'issues') {
    return null;
  }
  if (!UUID_RE.test(projectId) || !UUID_RE.test(issueId)) {
    return null;
  }

  return {
    origin: url.origin,
    projectId,
    issueId,
    url: `${url.origin}${url.pathname}`,
  };
}

export function findVibeIssueUrls(text: string): VibeIssueUrl[] {
  return Array.from(text.matchAll(URL_RE), ([candidate]) =>
    parseVibeIssueUrl(candidate)
  ).filter((issueUrl): issueUrl is VibeIssueUrl => issueUrl !== null);
}

function trimTrailingPunctuation(value: string): string {
  return value.replace(/[.,;:!?]+$/g, '');
}
