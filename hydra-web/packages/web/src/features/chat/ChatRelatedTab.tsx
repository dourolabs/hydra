import { Link } from "react-router-dom";
import { Spinner } from "@hydra/ui";
import type {
  DocumentSummaryRecord,
  IssueSummaryRecord,
  PatchSummaryRecord,
} from "@hydra/api";
import { ItemRow } from "../dashboard/ItemRow";
import type { WorkItem } from "../dashboard/workItemTypes";
import { TERMINAL_STATUSES } from "../../utils/statusMapping";
import { DocumentIcon } from "../../components/icons/DocumentIcon";
import { formatRelativeTime } from "../../utils/time";
import { useChatReferencedArtifacts } from "./useChatReferencedArtifacts";
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

interface ChatRelatedTabProps {
  conversationId: string;
}

export function ChatRelatedTab({ conversationId }: ChatRelatedTabProps) {
  const { issues, patches, documents, sessionsByIssue, isLoading, error } =
    useChatReferencedArtifacts(conversationId);

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
        <p className={styles.empty}>Failed to load referenced items.</p>
      </div>
    );
  }

  return (
    <div className={styles.relatedTab}>
      <Section title="Issues" count={issues.length}>
        {issues.length === 0 ? (
          <p className={styles.empty}>No issues referenced by this chat yet.</p>
        ) : (
          <ul className={styles.list}>
            {issues.map((record) => (
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
          <p className={styles.empty}>No patches referenced by this chat yet.</p>
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
          <p className={styles.empty}>No documents referenced by this chat yet.</p>
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
