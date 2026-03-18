import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { Spinner } from "@hydra/ui";
import type { IssueSummaryRecord } from "@hydra/api";
import { ItemRow } from "../dashboard/ItemRow";
import type { WorkItem } from "../dashboard/workItemTypes";
import { TERMINAL_STATUSES } from "../../utils/statusMapping";
import { useIssue } from "./useIssue";
import { topologicalSort } from "./topologicalSort";
import { apiClient } from "../../api/client";
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
  // Fetch the current issue to get its parent dependencies
  const { data: currentIssue } = useIssue(issueId);

  // Get parent IDs from the current issue's child-of dependencies
  const parentIds = useMemo(
    () =>
      currentIssue?.issue.dependencies
        .filter((dep) => dep.type === "child-of")
        .map((dep) => dep.issue_id) ?? [],
    [currentIssue],
  );

  // Fetch direct children via relationships API
  const { data: childRelations } = useQuery({
    queryKey: ["relations", "child-of", issueId],
    queryFn: () =>
      apiClient.listRelations({
        target_ids: issueId,
        rel_type: "child-of",
      }),
    staleTime: 30_000,
    select: (data) => data.relations,
  });

  const childIds = useMemo(
    () => childRelations?.map((rel) => rel.source_id) ?? [],
    [childRelations],
  );

  // Batch fetch all related issue details (parents + children)
  const allRelatedIds = useMemo(() => {
    const ids = new Set([...parentIds, ...childIds]);
    return Array.from(ids);
  }, [parentIds, childIds]);

  const idsParam = allRelatedIds.join(",");
  const { data: relatedIssues, isLoading } = useQuery({
    queryKey: ["issues", "batch", idsParam],
    queryFn: () => apiClient.listIssues({ ids: idsParam }),
    enabled: allRelatedIds.length > 0,
    staleTime: 30_000,
    select: (data) => data.issues,
  });

  // Fetch sessions for all related issues
  const spawned_from_ids = allRelatedIds.join(",");
  const { data: sessionsData } = useQuery({
    queryKey: ["sessions", "batch", spawned_from_ids],
    queryFn: () => apiClient.listSessions({ spawned_from_ids }),
    enabled: allRelatedIds.length > 0,
    staleTime: 30_000,
    select: (data) => data.sessions,
  });

  // Group sessions by issue ID
  const sessionsByIssue = useMemo(() => {
    const map = new Map<string, typeof sessionsData>();
    if (!sessionsData) return map;
    for (const session of sessionsData) {
      const sid = session.session.spawned_from;
      if (!sid) continue;
      const list = map.get(sid) ?? [];
      list.push(session);
      map.set(sid, list);
    }
    return map;
  }, [sessionsData]);

  if (isLoading && allRelatedIds.length > 0) {
    return <Spinner size="sm" />;
  }

  // Separate parents and children for display order
  const parentIdSet = new Set(parentIds);
  const childIdSet = new Set(childIds);
  const parents = relatedIssues?.filter((r) => parentIdSet.has(r.issue_id)) ?? [];
  const children = relatedIssues
    ? topologicalSort(relatedIssues.filter((r) => childIdSet.has(r.issue_id)))
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
