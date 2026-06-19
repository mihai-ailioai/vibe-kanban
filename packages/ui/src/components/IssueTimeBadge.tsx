'use client';

import { ClockIcon } from '@phosphor-icons/react';

import { cn } from '../lib/cn';

export interface IssueTimeBadgeProps {
  label: string;
  tooltip?: string;
  className?: string;
}

export function IssueTimeBadge({
  label,
  tooltip = 'Tracked OpenCode active time',
  className,
}: IssueTimeBadgeProps) {
  return (
    <span
      className={cn(
        'inline-flex items-center gap-half',
        'h-5 px-half',
        'rounded-sm bg-panel',
        'text-sm font-medium text-low',
        'whitespace-nowrap',
        className
      )}
      title={tooltip}
    >
      <ClockIcon aria-hidden className="size-icon-xs" weight="bold" />
      <span className="sr-only">Tracked OpenCode active time: </span>
      <span>{label}</span>
    </span>
  );
}
