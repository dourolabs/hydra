import { useMemo } from "react";
import { Panel, Spinner } from "@metis/ui";
import type { IssueSummaryRecord } from "@metis/api";
import { useIssues } from "../features/issues/useIssues";
import { useIssueFilters } from "../features/issues/useIssueFilters";
import { IssueTree } from "../features/issues/IssueTree";
import { IssueFilters } from "../features/issues/IssueFilters";
import { IssueCreator } from "../features/issues/IssueCreator";
import { useTreeExpandState } from "../features/issues/useTreeExpandState";
import { useAllJobs } from "../features/jobs/useAllJobs";
import type { IssueFilterValues } from "../features/issues/useIssueFilters";
import styles from "./IssuesPage.module.css";

function issueMatchesText(record: IssueSummaryRecord, q: string): boolean {
  const lower = q.toLowerCase();
  const desc = record.issue.description?.toLowerCase() ?? "";
  const id = record.issue_id.toLowerCase();
  return desc.includes(lower) || id.includes(lower);
}

function issueMatchesFilter(record: IssueSummaryRecord, statuses: string[], assignee: string, type: string, q: string): boolean {
  if (statuses.length > 0 && !statuses.includes(record.issue.status)) return false;
  if (assignee && record.issue.assignee !== assignee) return false;
  if (type && record.issue.type !== type) return false;
  if (q && !issueMatchesText(record, q)) return false;
  return true;
}

function getMatchingIds(issues: IssueSummaryRecord[], filters: IssueFilterValues): Set<string> {
  const ids = new Set<string>();
  for (const record of issues) {
    if (issueMatchesFilter(record, filters.statuses, filters.assignee, filters.type, filters.q)) {
      ids.add(record.issue_id);
    }
  }
  return ids;
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

export function IssuesPage() {
  const { filters, setFilters } = useIssueFilters();
  const { data: issues, isLoading, error } = useIssues();
  const { data: jobsByIssue } = useAllJobs();
  const { collapsedIds, onToggle } = useTreeExpandState();

  const assignees = useMemo(() => (issues ? extractAssignees(issues) : []), [issues]);

  const matchingIds = useMemo(
    () => (issues ? getMatchingIds(issues, filters) : new Set<string>()),
    [issues, filters],
  );

  const active = hasActiveFilters(filters);
  const issueList = issues ?? [];

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
        {issues && (issueList.length === 0 || (active && matchingIds.size === 0)) && (
          <p className={styles.empty}>
            {filters.q
              ? `No issues matching "${filters.q}".`
              : "No issues found."}
          </p>
        )}
        {issueList.length > 0 && (!active || matchingIds.size > 0) && (
          <IssueTree
            issues={issueList}
            matchingIds={active ? matchingIds : undefined}
            jobsByIssue={jobsByIssue}
            collapsedIds={collapsedIds}
            onToggle={onToggle}
          />
        )}
      </Panel>
    </div>
  );
}
