import { useMemo } from "react";
import { Panel, Spinner } from "@metis/ui";
import { useIssues } from "../features/issues/useIssues";
import { useIssueFilters } from "../features/issues/useIssueFilters";
import { IssueTree } from "../features/issues/IssueTree";
import { IssueFilters } from "../features/issues/IssueFilters";
import { IssueCreator } from "../features/issues/IssueCreator";
import type { Issue } from "../api/issues";
import type { IssueFilterValues, SortOption } from "../features/issues/useIssueFilters";
import styles from "./DashboardPage.module.css";

const STATUS_ORDER: Record<string, number> = {
  open: 0,
  "in-progress": 1,
  blocked: 2,
  failed: 3,
  closed: 4,
  dropped: 5,
  rejected: 6,
};

function issueMatchesFilter(issue: Issue, statuses: string[], assignee: string, type: string): boolean {
  if (statuses.length > 0 && !statuses.includes(issue.status)) return false;
  if (assignee && issue.assignee !== assignee) return false;
  if (type && issue.type !== type) return false;
  return true;
}

/** Return the set of issue IDs that directly match the current filters. */
function getMatchingIds(issues: Issue[], filters: IssueFilterValues): Set<string> {
  const ids = new Set<string>();
  for (const issue of issues) {
    if (issueMatchesFilter(issue, filters.statuses, filters.assignee, filters.type)) {
      ids.add(issue.issue_id);
    }
  }
  return ids;
}

function sortIssues(issues: Issue[], sort: SortOption): Issue[] {
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
      sorted.sort((a, b) => (STATUS_ORDER[a.status] ?? 99) - (STATUS_ORDER[b.status] ?? 99));
      break;
  }

  return sorted;
}

function extractAssignees(issues: Issue[]): string[] {
  const set = new Set<string>();
  for (const issue of issues) {
    if (issue.assignee) set.add(issue.assignee);
  }
  return Array.from(set).sort();
}

/** Check whether any filter is actively set. */
function hasActiveFilters(filters: IssueFilterValues): boolean {
  return filters.statuses.length > 0 || filters.assignee !== "" || filters.type !== "";
}

export function DashboardPage() {
  const { data: issues, isLoading, error } = useIssues();
  const { filters, setFilters } = useIssueFilters();

  const assignees = useMemo(() => (issues ? extractAssignees(issues) : []), [issues]);

  const sortedIssues = useMemo(() => {
    if (!issues) return [];
    return sortIssues(issues, filters.sort);
  }, [issues, filters.sort]);

  const matchingIds = useMemo(
    () => (issues ? getMatchingIds(issues, filters) : new Set<string>()),
    [issues, filters],
  );

  const active = hasActiveFilters(filters);

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
        {issues && (sortedIssues.length === 0 || (active && matchingIds.size === 0)) && (
          <p className={styles.empty}>No issues found.</p>
        )}
        {sortedIssues.length > 0 && (!active || matchingIds.size > 0) && (
          <IssueTree
            issues={sortedIssues}
            matchingIds={active ? matchingIds : undefined}
          />
        )}
      </Panel>
    </div>
  );
}
