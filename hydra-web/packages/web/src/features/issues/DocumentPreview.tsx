import { useMemo } from "react";
import { Link } from "react-router-dom";
import { Spinner } from "@hydra/ui";
import { Markdown } from "../../components/Markdown";
import { useDocument } from "../documents/useDocument";
import styles from "./DocumentPreview.module.css";

interface DocumentPreviewCardProps {
  documentId: string;
}

function truncateBody(body: string, maxLines: number = 10): string {
  const lines = body.split("\n");
  if (lines.length <= maxLines) return body;
  return lines.slice(0, maxLines).join("\n") + "\n...";
}

function DocumentPreviewCard({ documentId }: DocumentPreviewCardProps) {
  const { data: record, isLoading, error } = useDocument(documentId);

  const truncatedBody = useMemo(
    () => record ? truncateBody(record.document.body_markdown) : "",
    [record],
  );

  if (isLoading) {
    return (
      <div className={styles.documentCard}>
        <Spinner size="sm" />
      </div>
    );
  }

  if (error || !record) {
    return (
      <div className={styles.documentCard}>
        <p className={styles.error}>
          Failed to load document {documentId}
        </p>
      </div>
    );
  }

  const { document } = record;

  return (
    <div className={styles.documentCard}>
      <div className={styles.documentHeader}>
        <Link
          to={`/documents/${record.document_id}`}
          className={styles.documentLink}
        >
          {document.title || document.path || record.document_id}
        </Link>
      </div>

      {document.path && (
        <span className={styles.documentPath}>{document.path}</span>
      )}

      {document.body_markdown && (
        <div className={styles.bodyPreview}>
          <Markdown content={truncatedBody} />
        </div>
      )}
    </div>
  );
}

interface DocumentPreviewProps {
  documentIds: string[];
}

export function DocumentPreview({ documentIds }: DocumentPreviewProps) {
  if (documentIds.length === 0) {
    return null;
  }

  return (
    <div className={styles.container}>
      {documentIds.map((documentId) => (
        <DocumentPreviewCard key={documentId} documentId={documentId} />
      ))}
    </div>
  );
}
