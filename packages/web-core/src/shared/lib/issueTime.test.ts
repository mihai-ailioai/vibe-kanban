import { describe, expect, it } from 'vitest';

import { formatIssueActiveTime, getIssueTotalMs } from './issueTime';

describe('issueTime', () => {
  describe('formatIssueActiveTime', () => {
    it('formats active time totals for issue badges', () => {
      expect(formatIssueActiveTime(-1)).toBeNull();
      expect(formatIssueActiveTime(0)).toBeNull();
      expect(formatIssueActiveTime(1)).toBe('<1m');
      expect(formatIssueActiveTime(59_999)).toBe('<1m');
      expect(formatIssueActiveTime(60_000)).toBe('1m');
      expect(formatIssueActiveTime(12 * 60_000)).toBe('12m');
      expect(formatIssueActiveTime(60 * 60_000)).toBe('1h');
      expect(formatIssueActiveTime(80 * 60_000)).toBe('1h 20m');
      expect(formatIssueActiveTime(24 * 60 * 60_000)).toBe('1d');
      expect(formatIssueActiveTime((2 * 24 + 3) * 60 * 60_000)).toBe('2d 3h');
    });
  });

  describe('getIssueTotalMs', () => {
    it('normalizes optional issue time total values to a number', () => {
      expect(getIssueTotalMs()).toBe(0);
      expect(getIssueTotalMs(null)).toBe(0);
      expect(getIssueTotalMs({ total_ms: 123 })).toBe(123);
      expect(getIssueTotalMs({ total_ms: '456' })).toBe(456);
      expect(getIssueTotalMs({ total_ms: 789n })).toBe(789);
      expect(getIssueTotalMs({ total_ms: -123 })).toBe(-123);
    });

    it('returns zero for invalid, non-finite, or unsafe issue time totals', () => {
      expect(getIssueTotalMs({ total_ms: 'invalid' })).toBe(0);
      expect(getIssueTotalMs({ total_ms: '' })).toBe(0);
      expect(getIssueTotalMs({ total_ms: Number.NaN })).toBe(0);
      expect(getIssueTotalMs({ total_ms: Number.POSITIVE_INFINITY })).toBe(0);
      expect(getIssueTotalMs({ total_ms: Number.MAX_SAFE_INTEGER + 1 })).toBe(
        0
      );
      expect(getIssueTotalMs({ total_ms: `${Number.MAX_SAFE_INTEGER}1` })).toBe(
        0
      );
      expect(
        getIssueTotalMs({ total_ms: BigInt(Number.MAX_SAFE_INTEGER) + 1n })
      ).toBe(0);
    });
  });
});
