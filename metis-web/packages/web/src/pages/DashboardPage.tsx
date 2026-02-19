import { useMemo } from "react";
import { Panel, Spinner } from "@metis/ui";
import { useIssues } from "../features/issues/useIssues";
import { useIssueFilters } from "../features/issues/useIssueFilters";
import { IssueTree } from "../features/issues/IssueTree";
import { IssueFilters } from "../features/issues/IssueFilters";
import { IssueCreator } from "../features/issues/IssueCreator";
import type { Issue } from "../api/issues";
import type { SortOption } from "../features/issues/useIssueFilters";
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

function filterIssues(issues: Issue[], statuses: string[], assignee: string, type: string): Issue[] {
  let result = issues;

  if (statuses.length > 0) {
    const statusSet = new Set(statuses);
    result = result.filter((i) => statusSet.has(i.status));
  }

  if (assignee) {
    result = result.filter((i) => i.assignee === assignee);
  }

  if (type) {
    result = result.filter((i) => i.type === type);
  }

  return result;
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

export function DashboardPage() {
  const { data: issues, isLoading, error } = useIssues();
  const { filters, setFilters } = useIssueFilters();

  const assignees = useMemo(() => (issues ? extractAssignees(issues) : []), [issues]);

  const filteredIssues = useMemo(() => {
    if (!issues) return [];
    const filtered = filterIssues(issues, filters.statuses, filters.assignee, filters.type);
    return sortIssues(filtered, filters.sort);
  }, [issues, filters]);

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
        {issues && filteredIssues.length === 0 && (
          <p className={styles.empty}>No issues found.</p>
        )}
        {filteredIssues.length > 0 && <IssueTree issues={filteredIssues} />}
      </Panel>
    </div>
  );
}
