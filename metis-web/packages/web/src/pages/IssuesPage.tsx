import { useMemo } from "react";
import { Panel, Spinner } from "@metis/ui";
import type { IssueSummaryRecord } from "@metis/api";
import { useIssues } from "../features/issues/useIssues";
import type { IssueFilterParams } from "../features/issues/useIssues";
import { useIssueFilters } from "../features/issues/useIssueFilters";
import { IssueTree } from "../features/issues/IssueTree";
import { IssueFilters } from "../features/issues/IssueFilters";
import { IssueCreator } from "../features/issues/IssueCreator";
import { useTreeExpandState } from "../features/issues/useTreeExpandState";
import { useAllJobs } from "../features/jobs/useAllJobs";
import type { IssueFilterValues, SortOption } from "../features/issues/useIssueFilters";
import styles from "./IssuesPage.module.css";

const STATUS_ORDER: Record<string, number> = {
  open: 0,
  "in-progress": 1,
  blocked: 2,
  failed: 3,
  closed: 4,
  dropped: 5,
  rejected: 6,
};

function sortIssues(issues: IssueSummaryRecord[], sort: SortOption): IssueSummaryRecord[] {
  const sorted = [...issues];

  switch (sort) {
    case "newest":
      sorted.sort((a, b) => b.timestamp.localeCompare(a.timestamp));
      break;
    case "oldest":
      sorted.sort((a, b) => a.timestamp.localeCompare(b.timestamp));
      break;
    case "updated":
      sorted.sort((a, b) => b.timestamp.localeCompare(a.timestamp));
      break;
    case "status":
      sorted.sort((a, b) => (STATUS_ORDER[a.issue.status] ?? 99) - (STATUS_ORDER[b.issue.status] ?? 99));
      break;
  }

  return sorted;
}

function extractAssignees(issues: IssueSummaryRecord[]): string[] {
  const set = new Set<string>();
  for (const record of issues) {
    if (record.issue.assignee) set.add(record.issue.assignee);
  }
  return Array.from(set).sort();
}

function hasActiveFilters(filters: IssueFilterValues): boolean {
  return filters.statuses.length > 0 || filters.assignee !== "" || filters.type !== "" || filters.q !== "";
}

/** Build server-side filter params from the current UI filter state. */
function buildFilterParams(filters: IssueFilterValues): IssueFilterParams | undefined {
  const params: IssueFilterParams = {};
  if (filters.statuses.length === 1) params.status = filters.statuses[0];
  if (filters.assignee) params.assignee = filters.assignee;
  if (filters.type) params.issue_type = filters.type;
  if (filters.q) params.q = filters.q;
  return Object.keys(params).length > 0 ? params : undefined;
}

export function IssuesPage() {
  const { filters, setFilters } = useIssueFilters();
  const active = hasActiveFilters(filters);
  const filterParams = useMemo(() => buildFilterParams(filters), [filters]);
  const { data, isLoading, error } = useIssues(filterParams);
  const { data: jobsByIssue } = useAllJobs();
  const { collapsedIds, onToggle } = useTreeExpandState();

  const issues = data?.issues;
  const serverMatchingIds = data?.matchingIds;

  const assignees = useMemo(() => (issues ? extractAssignees(issues) : []), [issues]);

  const sortedIssues = useMemo(() => {
    if (!issues) return [];
    return sortIssues(issues, filters.sort);
  }, [issues, filters.sort]);

  // Use server-provided matching_ids when filters are active.
  // For multi-status filters (not yet supported server-side), fall back to
  // treating all returned issues as matching.
  const matchingIds = useMemo(() => {
    if (!active) return undefined;
    if (serverMatchingIds) return new Set<string>(serverMatchingIds);
    // Fallback: all returned issues are considered matching
    if (issues) return new Set<string>(issues.map((i) => i.issue_id));
    return new Set<string>();
  }, [active, serverMatchingIds, issues]);

  return (
    <div className={styles.page}>
      <IssueCreator assignees={assignees} />
      <Panel
        header={
          <IssueFilters
            filters={filters}
            assignees={assignees}
            onFilterChange={setFilters}
          />
        }
      >
        {isLoading && (
          <div className={styles.center}>
            <Spinner size="md" />
          </div>
        )}
        {error && (
          <p className={styles.error}>Failed to load issues: {(error as Error).message}</p>
        )}
        {issues && (sortedIssues.length === 0 || (active && matchingIds && matchingIds.size === 0)) && (
          <p className={styles.empty}>
            {filters.q
              ? `No issues matching "${filters.q}".`
              : "No issues found."}
          </p>
        )}
        {sortedIssues.length > 0 && (!active || (matchingIds && matchingIds.size > 0)) && (
          <IssueTree
            issues={sortedIssues}
            matchingIds={matchingIds}
            jobsByIssue={jobsByIssue}
            collapsedIds={collapsedIds}
            onToggle={onToggle}
          />
        )}
      </Panel>
    </div>
  );
}
