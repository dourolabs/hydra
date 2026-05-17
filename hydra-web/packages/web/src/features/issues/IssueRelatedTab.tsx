import { useMemo } from "react";
import { Link } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { Spinner } from "@hydra/ui";
import type {
  DocumentSummaryRecord,
  IssueSummaryRecord,
  PatchSummaryRecord,
  SessionSummaryRecord,
} from "@hydra/api";
import { ItemRow } from "../dashboard/ItemRow";
import type { WorkItem } from "../dashboard/workItemTypes";
import { TERMINAL_STATUSES } from "../../utils/statusMapping";
import { DocumentIcon } from "../../components/icons/DocumentIcon";
import { formatRelativeTime } from "../../utils/time";
import { apiClient } from "../../api/client";
import { useIssue } from "./useIssue";
import { useIssuePatches } from "../patches/useIssuePatches";
import { useIssueDocuments } from "./useIssueDocuments";
import { topologicalSort } from "./topologicalSort";
import styles from "./IssueRelatedTab.module.css";

function issueToWorkItem(record: IssueSummaryRecord): WorkItem {
  return {
    kind: "issue",
    id: record.issue_id,
    data: record,
    lastUpdated: record.timestamp,
    isTerminal: TERMINAL_STATUSES.has(record.issue.status),
  };
}

function patchToWorkItem(record: PatchSummaryRecord): WorkItem {
  return {
    kind: "patch",
    id: record.patch_id,
    data: record,
    lastUpdated: record.timestamp,
    isTerminal: record.patch.status === "Closed" || record.patch.status === "Merged",
    sourceIssueId: undefined,
  };
}

function getDocumentTitle(doc: DocumentSummaryRecord): string {
  if (doc.document.title) return doc.document.title;
  if (doc.document.path) return doc.document.path;
  return doc.document_id;
}

interface SectionProps {
  title: string;
  count: number | null;
  children: React.ReactNode;
}

function Section({ title, count, children }: SectionProps) {
  return (
    <section className={styles.section}>
      <h3 className={styles.sectionTitle}>
        {title}
        {count !== null && <span className={styles.sectionCount}>({count})</span>}
      </h3>
      {children}
    </section>
  );
}

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
    select: (data) => data.issues,
  });

  // Mirror IssueRelatedIssues queryKey shape so useSSE invalidation on
  // session_* events refreshes the running-session indicator live.
  const sessionsQuery = useQuery({
    queryKey: ["sessions", "batch", idsParam],
    queryFn: () => apiClient.listSessions({ spawned_from_ids: idsParam }),
    enabled: allRelatedIds.length > 0,
    staleTime: 30_000,
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
        <p className={styles.empty}>Failed to load related items.</p>
      </div>
    );
  }

  return (
    <div className={styles.relatedTab}>
      <Section title="Parents" count={parents.length}>
        {parents.length === 0 ? (
          <p className={styles.empty}>No parent issues.</p>
        ) : (
          <ul className={styles.list}>
            {parents.map((record) => (
              <ItemRow
                key={record.issue_id}
                item={issueToWorkItem(record)}
                sessions={sessionsByIssue.get(record.issue_id)}
                filterRootId={null}
              />
            ))}
          </ul>
        )}
      </Section>

      <Section title="Children" count={children.length}>
        {children.length === 0 ? (
          <p className={styles.empty}>No child issues.</p>
        ) : (
          <ul className={styles.list}>
            {children.map((record) => (
              <ItemRow
                key={record.issue_id}
                item={issueToWorkItem(record)}
                sessions={sessionsByIssue.get(record.issue_id)}
                filterRootId={null}
              />
            ))}
          </ul>
        )}
      </Section>

      <Section title="Patches" count={patches.length}>
        {patches.length === 0 ? (
          <p className={styles.empty}>No patches linked to this issue.</p>
        ) : (
          <ul className={styles.list}>
            {patches.map((record) => (
              <ItemRow
                key={record.patch_id}
                item={patchToWorkItem(record)}
                filterRootId={null}
              />
            ))}
          </ul>
        )}
      </Section>

      <Section title="Documents" count={documents.length}>
        {documents.length === 0 ? (
          <p className={styles.empty}>No documents linked to this issue.</p>
        ) : (
          <ul className={styles.list}>
            {documents.map((doc) => (
              <li key={doc.document_id} className={styles.docRow}>
                <Link to={`/documents/${doc.document_id}`} className={styles.docRowLink}>
                  <DocumentIcon className={styles.docIcon} />
                  <span className={styles.docTitle}>{getDocumentTitle(doc)}</span>
                  <span className={styles.docMeta}>
                    {doc.document.path && (
                      <span className={styles.docPath}>{doc.document.path}</span>
                    )}
                    <span className={styles.docTime}>
                      {formatRelativeTime(doc.timestamp)}
                    </span>
                  </span>
                </Link>
              </li>
            ))}
          </ul>
        )}
      </Section>
    </div>
  );
}
