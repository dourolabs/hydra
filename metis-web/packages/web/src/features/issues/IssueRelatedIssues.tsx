import { Spinner } from "@metis/ui";
import type { IssueSummaryRecord } from "@metis/api";
import { ItemRow } from "../dashboard/ItemRow";
import type { WorkItem } from "../dashboard/useTransitiveWorkItems";
import { TERMINAL_STATUSES } from "../../utils/statusMapping";
import { useIssues } from "./useIssues";
import { useAllSessions } from "../sessions/useAllSessions";
import { topologicalSort } from "./topologicalSort";
import styles from "./IssueRelatedIssues.module.css";

function toWorkItem(record: IssueSummaryRecord): WorkItem {
  return {
    kind: "issue",
    id: record.issue_id,
    data: record,
    lastUpdated: record.timestamp,
    isTerminal: TERMINAL_STATUSES.has(record.issue.status),
  };
}

interface IssueRelatedIssuesProps {
  issueId: string;
}

export function IssueRelatedIssues({ issueId }: IssueRelatedIssuesProps) {
  const { data: allIssues, isLoading } = useIssues();
  const { data: sessionsByIssue } = useAllSessions();

  if (isLoading) {
    return <Spinner size="sm" />;
  }

  // Find current issue to read its parent dependencies
  const currentIssue = allIssues?.find((r) => r.issue_id === issueId);
  const parentIds =
    currentIssue?.issue.dependencies
      .filter((dep) => dep.type === "child-of")
      .map((dep) => dep.issue_id) ?? [];
  const parents = allIssues
    ? allIssues.filter((r) => parentIds.includes(r.issue_id))
    : [];

  // Find children: issues that have a "child-of" dependency on this issueId
  const children = allIssues
    ? topologicalSort(
        allIssues.filter((record) =>
          record.issue.dependencies.some(
            (dep) => dep.type === "child-of" && dep.issue_id === issueId,
          ),
        ),
      )
    : [];

  const allRelated = [...parents, ...children];

  if (allRelated.length === 0) {
    return (
      <div className={styles.empty}>
        <p className={styles.emptyText}>No related issues.</p>
      </div>
    );
  }

  return (
    <ul className={styles.list}>
      {allRelated.map((record) => (
        <ItemRow key={record.issue_id} item={toWorkItem(record)} sessions={sessionsByIssue?.get(record.issue_id)} />
      ))}
    </ul>
  );
}
