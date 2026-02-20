import { useState, useMemo, useCallback } from "react";
import { Panel, Spinner, MarkdownViewer } from "@metis/ui";
import type { DocumentVersionRecord } from "@metis/api";
import { useDocuments } from "../features/documents/useDocuments";
import { useDocument } from "../features/documents/useDocument";
import { formatRelativeTime } from "../utils/time";
import styles from "./DocumentsPage.module.css";

interface DocumentGroup {
  prefix: string;
  documents: DocumentVersionRecord[];
}

function getPathPrefix(doc: DocumentVersionRecord): string {
  const path = doc.document.path;
  if (!path) return "";
  // Strip leading slash, then take the first path segment
  const cleaned = path.startsWith("/") ? path.slice(1) : path;
  const slashIndex = cleaned.indexOf("/");
  if (slashIndex < 0) return "";
  return cleaned.slice(0, slashIndex);
}

function groupDocumentsByPrefix(documents: DocumentVersionRecord[]): DocumentGroup[] {
  const groups = new Map<string, DocumentVersionRecord[]>();

  for (const doc of documents) {
    if (doc.document.deleted) continue;
    const prefix = getPathPrefix(doc);
    const list = groups.get(prefix) ?? [];
    list.push(doc);
    groups.set(prefix, list);
  }

  // Sort groups alphabetically, with uncategorized ("") last
  const sorted: DocumentGroup[] = [];
  const keys = Array.from(groups.keys()).sort((a, b) => {
    if (a === "") return 1;
    if (b === "") return -1;
    return a.localeCompare(b);
  });

  for (const key of keys) {
    sorted.push({ prefix: key, documents: groups.get(key)! });
  }

  return sorted;
}

function getDocumentDisplayTitle(doc: DocumentVersionRecord): string {
  if (doc.document.title) return doc.document.title;
  if (doc.document.path) return doc.document.path;
  return doc.document_id;
}

export function DocumentsPage() {
  const { data: documents, isLoading, error } = useDocuments();
  const [expandedId, setExpandedId] = useState<string | null>(null);

  const groups = useMemo(() => (documents ? groupDocumentsByPrefix(documents) : []), [documents]);

  const handleToggle = useCallback((documentId: string) => {
    setExpandedId((prev) => (prev === documentId ? null : documentId));
  }, []);

  return (
    <div className={styles.page}>
      {isLoading && (
        <div className={styles.center}>
          <Spinner size="md" />
        </div>
      )}

      {error && (
        <p className={styles.error}>Failed to load documents: {(error as Error).message}</p>
      )}

      {documents && groups.length === 0 && <p className={styles.empty}>No documents found.</p>}

      {groups.map((group) => (
        <Panel
          key={group.prefix || "__uncategorized"}
          header={<span className={styles.sectionTitle}>{group.prefix || "Uncategorized"}</span>}
        >
          <ul className={styles.docList}>
            {group.documents.map((doc) => (
              <DocumentRow
                key={doc.document_id}
                doc={doc}
                expanded={expandedId === doc.document_id}
                onToggle={handleToggle}
              />
            ))}
          </ul>
        </Panel>
      ))}
    </div>
  );
}

interface DocumentRowProps {
  doc: DocumentVersionRecord;
  expanded: boolean;
  onToggle: (id: string) => void;
}

function DocumentRow({ doc, expanded, onToggle }: DocumentRowProps) {
  return (
    <li>
      <div
        className={`${styles.docRow} ${expanded ? styles.docRowExpanded : ""}`}
        onClick={() => onToggle(doc.document_id)}
        role="button"
        tabIndex={0}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            onToggle(doc.document_id);
          }
        }}
      >
        <span className={styles.expandIcon}>{expanded ? "\u25BC" : "\u25B6"}</span>
        <span className={styles.docTitle}>{getDocumentDisplayTitle(doc)}</span>
        {doc.document.path && <span className={styles.docPath}>{doc.document.path}</span>}
        <span className={styles.docTime}>{formatRelativeTime(doc.timestamp)}</span>
      </div>
      {expanded && <DocumentContent documentId={doc.document_id} />}
    </li>
  );
}

interface DocumentContentProps {
  documentId: string;
}

function DocumentContent({ documentId }: DocumentContentProps) {
  const { data: record, isLoading, error } = useDocument(documentId);

  if (isLoading) {
    return (
      <div className={styles.docContentCenter}>
        <Spinner size="sm" />
      </div>
    );
  }

  if (error) {
    return (
      <div className={styles.docContent}>
        <p className={styles.error}>Failed to load document: {(error as Error).message}</p>
      </div>
    );
  }

  if (!record) return null;

  return (
    <div className={styles.docContent}>
      <MarkdownViewer content={record.document.body_markdown} />
    </div>
  );
}
