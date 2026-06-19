const MINUTE_MS = 60_000;
const HOUR_MS = 60 * MINUTE_MS;
const DAY_MS = 24 * HOUR_MS;
const MAX_SAFE_INTEGER_BIGINT = BigInt(Number.MAX_SAFE_INTEGER);
const MIN_SAFE_INTEGER_BIGINT = BigInt(Number.MIN_SAFE_INTEGER);

export function formatIssueActiveTime(totalMs: number): string | null {
  if (totalMs <= 0) {
    return null;
  }

  if (totalMs < MINUTE_MS) {
    return '<1m';
  }

  const days = Math.floor(totalMs / DAY_MS);
  if (days > 0) {
    const hours = Math.floor((totalMs % DAY_MS) / HOUR_MS);
    return hours > 0 ? `${days}d ${hours}h` : `${days}d`;
  }

  const hours = Math.floor(totalMs / HOUR_MS);
  if (hours > 0) {
    const minutes = Math.floor((totalMs % HOUR_MS) / MINUTE_MS);
    return minutes > 0 ? `${hours}h ${minutes}m` : `${hours}h`;
  }

  return `${Math.floor(totalMs / MINUTE_MS)}m`;
}

export function getIssueTotalMs(
  total?: { total_ms: bigint | number | string } | null
): number {
  if (!total) {
    return 0;
  }

  const { total_ms: totalMs } = total;

  if (
    typeof totalMs === 'bigint' &&
    (totalMs > MAX_SAFE_INTEGER_BIGINT || totalMs < MIN_SAFE_INTEGER_BIGINT)
  ) {
    return 0;
  }

  if (typeof totalMs === 'string' && totalMs.trim() === '') {
    return 0;
  }

  const convertedTotalMs = Number(totalMs);

  return Number.isSafeInteger(convertedTotalMs) ? convertedTotalMs : 0;
}
