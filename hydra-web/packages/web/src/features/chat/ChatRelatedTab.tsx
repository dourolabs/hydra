import { useMemo } from "react";
import { Link } from "react-router-dom";
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
import { getDocumentDisplayTitle } from "../documents/utils";
import { formatRelativeTime } from "../../utils/time";
import { useChatActiveSessionIssues } from "./useChatActiveSessionIssues";
import { useChatAttentionIssues } from "./useChatAttentionIssues";
import { useChatTopLevelIssues } from "./useChatTopLevelIssues";
import { useChatRelatedDocuments } from "./useChatRelatedDocuments";
import { useChatRelatedPatches } from "./useChatRelatedPatches";
import styles from "./ChatRelatedTab.module.css";

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

interface SectionProps {
  title: string;
  count: number | null;
  isLoading: boolean;
  children: React.ReactNode;
}

function Section({ title, count, isLoading, children }: SectionProps) {
  return (
    <section className={styles.section}>
      <h3 className={styles.sectionTitle}>
        {title}
        {count !== null && <span className={styles.sectionCount}>({count})</span>}
      </h3>
      {isLoading ? (
        <div className={styles.spinnerWrapper}>
          <Spinner size="sm" />
        </div>
      ) : (
        children
      )}
    </section>
  );
}

interface IssueListProps {
  issues: IssueSummaryRecord[];
  sessionsByIssue?: Map<string, SessionSummaryRecord[]>;
  forceActive?: boolean;
}

function IssueList({ issues, sessionsByIssue, forceActive }: IssueListProps) {
  if (issues.length === 0) {
    return <p className={styles.empty}>(empty)</p>;
  }
  return (
    <ul className={styles.list}>
      {issues.map((record) => (
        <ItemRow
          key={record.issue_id}
          item={issueToWorkItem(record)}
          sessions={sessionsByIssue?.get(record.issue_id)}
          isActive={forceActive || undefined}
          filterRootId={null}
        />
      ))}
    </ul>
  );
}

function DocumentList({ documents }: { documents: DocumentSummaryRecord[] }) {
  if (documents.length === 0) {
    return <p className={styles.empty}>(empty)</p>;
  }
  return (
    <ul className={styles.list}>
      {documents.map((doc) => (
        <li key={doc.document_id} className={styles.docRow}>
          <Link to={`/documents/${doc.document_id}`} className={styles.docRowLink}>
            <DocumentIcon className={styles.docIcon} />
            <span className={styles.docTitle}>{getDocumentDisplayTitle(doc)}</span>
            <span className={styles.docMeta}>
              {doc.document.path && (
                <span className={styles.docPath}>{doc.document.path}</span>
              )}
              <span className={styles.docTime}>{formatRelativeTime(doc.timestamp)}</span>
            </span>
          </Link>
        </li>
      ))}
    </ul>
  );
}

function PatchList({ patches }: { patches: PatchSummaryRecord[] }) {
  if (patches.length === 0) {
    return <p className={styles.empty}>(empty)</p>;
  }
  return (
    <ul className={styles.list}>
      {patches.map((record) => (
        <ItemRow
          key={record.patch_id}
          item={patchToWorkItem(record)}
          filterRootId={null}
        />
      ))}
    </ul>
  );
}

export function ChatRelatedTab() {
  const activeSessions = useChatActiveSessionIssues();

  const activeIds = useMemo(
    () => new Set(activeSessions.issues.map((i) => i.issue_id)),
    [activeSessions.issues],
  );
  const attention = useChatAttentionIssues(activeIds);

  const topLevelExcludeIds = useMemo(() => {
    const set = new Set(activeIds);
    for (const issue of attention.issues) set.add(issue.issue_id);
    return set;
  }, [activeIds, attention.issues]);
  const topLevel = useChatTopLevelIssues(topLevelExcludeIds);

  const documents = useChatRelatedDocuments();
  const patches = useChatRelatedPatches();

  return (
    <div className={styles.relatedTab}>
      <Section
        title="Issues with active sessions"
        count={activeSessions.isLoading ? null : activeSessions.issues.length}
        isLoading={activeSessions.isLoading}
      >
        <IssueList
          issues={activeSessions.issues}
          sessionsByIssue={activeSessions.sessionsByIssue}
          forceActive
        />
      </Section>

      <Section
        title="Needs my attention"
        count={attention.isLoading ? null : attention.issues.length}
        isLoading={attention.isLoading}
      >
        <IssueList issues={attention.issues} />
      </Section>

      <Section
        title="Top-level issues"
        count={topLevel.isLoading ? null : topLevel.issues.length}
        isLoading={topLevel.isLoading}
      >
        <IssueList issues={topLevel.issues} />
      </Section>

      <Section
        title="Documents"
        count={documents.isLoading ? null : documents.documents.length}
        isLoading={documents.isLoading}
      >
        <DocumentList documents={documents.documents} />
      </Section>

      <Section
        title="Patches"
        count={patches.isLoading ? null : patches.patches.length}
        isLoading={patches.isLoading}
      >
        <PatchList patches={patches.patches} />
      </Section>
    </div>
  );
}
