import { useMemo } from "react";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { Spinner } from "@hydra/ui";
import type { SessionSummaryRecord } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useIssue } from "./useIssue";
import { useIssuePatches } from "../patches/useIssuePatches";
import { useIssueDocuments } from "./useIssueDocuments";
import { topologicalSort } from "./topologicalSort";
import { RelatedSection, RelatedEmpty } from "../related/RelatedSection";
import {
  IssueRailRow,
  PatchRailRow,
  DocumentRailRow,
} from "../related/RailRow";
import { usePageIssueTrees } from "../dashboard/usePageIssueTrees";
import styles from "./IssueRelatedTab.module.css";

interface IssueRelatedTabProps {
  issueId: string;
}

export function IssueRelatedTab({ issueId }: IssueRelatedTabProps) {
  const { data: currentIssue, isLoading: issueLoading } = useIssue(issueId);

  const parentIds = useMemo(
    () =>
      currentIssue?.issue.dependencies
        .filter((dep) => dep.type === "child-of")
        .map((dep) => dep.issue_id) ?? [],
    [currentIssue],
  );

  const childRelationsQuery = useQuery({
    queryKey: ["relations", "child-of", issueId],
    queryFn: () =>
      apiClient.listRelations({
        target_ids: issueId,
        rel_type: "child-of",
      }),
    enabled: !!issueId,
    staleTime: 30_000,
    placeholderData: keepPreviousData,
    select: (data) => data.relations,
  });

  const childIds = useMemo(
    () => childRelationsQuery.data?.map((rel) => rel.source_id) ?? [],
    [childRelationsQuery.data],
  );

  const allRelatedIds = useMemo(() => {
    const ids = new Set<string>([...parentIds, ...childIds]);
    return Array.from(ids);
  }, [parentIds, childIds]);

  const idsParam = allRelatedIds.join(",");
  const relatedIssuesQuery = useQuery({
    queryKey: ["issues", "batch", idsParam],
    queryFn: () => apiClient.listIssues({ ids: idsParam }),
    enabled: allRelatedIds.length > 0,
    staleTime: 30_000,
    placeholderData: keepPreviousData,
    select: (data) => data.issues,
  });

  // Mirror IssueRelatedIssues queryKey shape so useSSE invalidation on
  // session_* events refreshes the running-session indicator live.
  const sessionsQuery = useQuery({
    queryKey: ["sessions", "batch", idsParam],
    queryFn: () => apiClient.listSessions({ spawned_from_ids: idsParam }),
    enabled: allRelatedIds.length > 0,
    staleTime: 30_000,
    placeholderData: keepPreviousData,
    select: (data) => data.sessions,
  });

  const sessionsByIssue = useMemo(() => {
    const map = new Map<string, SessionSummaryRecord[]>();
    for (const session of sessionsQuery.data ?? []) {
      const sid = session.session.spawned_from;
      if (!sid) continue;
      const list = map.get(sid) ?? [];
      list.push(session);
      map.set(sid, list);
    }
    return map;
  }, [sessionsQuery.data]);

  const { data: patches, isLoading: patchesLoading, error: patchesError } =
    useIssuePatches(issueId);
  const { data: documents, isLoading: documentsLoading, error: documentsError } =
    useIssueDocuments(issueId);

  const parentIdSet = useMemo(() => new Set(parentIds), [parentIds]);
  const childIdSet = useMemo(() => new Set(childIds), [childIds]);

  const parents = useMemo(
    () =>
      (relatedIssuesQuery.data ?? []).filter((r) => parentIdSet.has(r.issue_id)),
    [relatedIssuesQuery.data, parentIdSet],
  );
  const children = useMemo(
    () =>
      topologicalSort(
        (relatedIssuesQuery.data ?? []).filter((r) => childIdSet.has(r.issue_id)),
      ),
    [relatedIssuesQuery.data, childIdSet],
  );

  // Hydrate per-card progress data (child statuses + active-session-in-subtree
  // glow) the same way the issues list does so progress bars render here.
  // Username is unused by IssueRailRow's progress UI, so leave it empty.
  const relatedIssuesForTrees = useMemo(
    () => [...parents, ...children],
    [parents, children],
  );
  const { childStatusMap } = usePageIssueTrees(relatedIssuesForTrees, "");

  const isLoading =
    issueLoading ||
    childRelationsQuery.isLoading ||
    (allRelatedIds.length > 0 && relatedIssuesQuery.isLoading) ||
    (allRelatedIds.length > 0 && sessionsQuery.isLoading) ||
    patchesLoading ||
    documentsLoading;

  const error =
    childRelationsQuery.error ??
    relatedIssuesQuery.error ??
    sessionsQuery.error ??
    patchesError ??
    documentsError ??
    null;

  if (isLoading) {
    return (
      <div className={styles.relatedTab}>
        <div className={styles.spinnerWrapper}>
          <Spinner size="sm" />
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className={styles.relatedTab}>
        <p className={styles.error}>Failed to load related items.</p>
      </div>
    );
  }

  return (
    <div className={styles.relatedTab}>
      <RelatedSection title="Parents" count={parents.length}>
        {parents.length === 0 ? (
          <RelatedEmpty>No parent issues.</RelatedEmpty>
        ) : (
          <div className={styles.list}>
            {parents.map((record) => (
              <IssueRailRow
                key={record.issue_id}
                record={record}
                sessions={sessionsByIssue.get(record.issue_id)}
                childStatuses={childStatusMap.get(record.issue_id)}
              />
            ))}
          </div>
        )}
      </RelatedSection>

      <RelatedSection title="Children" count={children.length}>
        {children.length === 0 ? (
          <RelatedEmpty>No child issues.</RelatedEmpty>
        ) : (
          <div className={styles.list}>
            {children.map((record) => (
              <IssueRailRow
                key={record.issue_id}
                record={record}
                sessions={sessionsByIssue.get(record.issue_id)}
                childStatuses={childStatusMap.get(record.issue_id)}
              />
            ))}
          </div>
        )}
      </RelatedSection>

      <RelatedSection title="Patches" count={patches.length}>
        {patches.length === 0 ? (
          <RelatedEmpty>No patches linked to this issue.</RelatedEmpty>
        ) : (
          <div className={styles.list}>
            {patches.map((record) => (
              <PatchRailRow key={record.patch_id} record={record} />
            ))}
          </div>
        )}
      </RelatedSection>

      <RelatedSection title="Documents" count={documents.length}>
        {documents.length === 0 ? (
          <RelatedEmpty>No documents linked to this issue.</RelatedEmpty>
        ) : (
          <div className={styles.list}>
            {documents.map((record) => (
              <DocumentRailRow key={record.document_id} record={record} />
            ))}
          </div>
        )}
      </RelatedSection>
    </div>
  );
}
