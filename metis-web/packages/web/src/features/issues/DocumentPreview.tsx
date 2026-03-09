import { useMemo } from "react";
import { Link } from "react-router-dom";
import { MarkdownViewer, Spinner } from "@metis/ui";
import { useDocumentByPath } from "../documents/useDocumentByPath";
import styles from "./DocumentPreview.module.css";

interface DocumentPreviewCardProps {
  path: string;
}

function truncateBody(body: string, maxLines: number = 10): string {
  const lines = body.split("\n");
  if (lines.length <= maxLines) return body;
  return lines.slice(0, maxLines).join("\n") + "\n...";
}

function DocumentPreviewCard({ path }: DocumentPreviewCardProps) {
  const { data: record, isLoading, error } = useDocumentByPath(path);

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
          Failed to load document {path}
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
          {document.title || path}
        </Link>
      </div>

      <span className={styles.documentPath}>{path}</span>

      {document.body_markdown && (
        <div className={styles.bodyPreview}>
          <MarkdownViewer content={truncatedBody} />
        </div>
      )}
    </div>
  );
}

interface DocumentPreviewProps {
  paths: string[];
}

export function DocumentPreview({ paths }: DocumentPreviewProps) {
  if (paths.length === 0) {
    return null;
  }

  return (
    <div className={styles.container}>
      {paths.map((path) => (
        <DocumentPreviewCard key={path} path={path} />
      ))}
    </div>
  );
}
