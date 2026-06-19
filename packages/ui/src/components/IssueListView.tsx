'use client';

import type { MouseEvent } from 'react';
import { cn } from '../lib/cn';
import type { KanbanAssigneeUser } from './KanbanAssignee';
import {
  IssueListSection,
  type IssueListSectionStatus,
} from './IssueListSection';
import type {
  IssueListRowIssue,
  IssueListRowRelationship,
  IssueListRowTag,
} from './IssueListRow';

export interface IssueListViewProps {
  statuses: IssueListSectionStatus[];
  items: Record<string, string[]>;
  issueMap: Record<string, IssueListRowIssue>;
  issueAssigneesMap: Record<string, KanbanAssigneeUser[]>;
  getTagObjectsForIssue: (issueId: string) => IssueListRowTag[];
  getResolvedRelationshipsForIssue?: (
    issueId: string
  ) => IssueListRowRelationship[];
  getIssueTimeLabel?: (issueId: string) => string | null;
  onIssueClick: (issueId: string, e: MouseEvent) => void;
  selectedIssueId: string | null;
  selectedIssueIds?: Set<string>;
  isMultiSelectActive?: boolean;
  onIssueCheckboxChange?: (issueId: string, checked: boolean) => void;
  className?: string;
}

export function IssueListView({
  statuses,
  items,
  issueMap,
  issueAssigneesMap,
  getTagObjectsForIssue,
  getResolvedRelationshipsForIssue,
  getIssueTimeLabel,
  onIssueClick,
  selectedIssueId,
  selectedIssueIds,
  isMultiSelectActive,
  onIssueCheckboxChange,
  className,
}: IssueListViewProps) {
  return (
    <div className={cn('flex flex-col h-full overflow-y-auto', className)}>
      {statuses.map((status) => (
        <IssueListSection
          key={status.id}
          status={status}
          issueIds={items[status.id] ?? []}
          issueMap={issueMap}
          issueAssigneesMap={issueAssigneesMap}
          getTagObjectsForIssue={getTagObjectsForIssue}
          getResolvedRelationshipsForIssue={getResolvedRelationshipsForIssue}
          getIssueTimeLabel={getIssueTimeLabel}
          onIssueClick={onIssueClick}
          selectedIssueId={selectedIssueId}
          selectedIssueIds={selectedIssueIds}
          isMultiSelectActive={isMultiSelectActive}
          onIssueCheckboxChange={onIssueCheckboxChange}
        />
      ))}
    </div>
  );
}
