import { useParams } from "react-router-dom";
import { Spinner } from "@hydra/ui";
import { ApiError } from "../api/client";
import { useDocument } from "../features/documents/useDocument";
import { DocumentDetail } from "../features/documents/DocumentDetail";
import type { BreadcrumbItem } from "../layout/Breadcrumbs";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import styles from "./DocumentDetailPage.module.css";

export function DocumentDetailPage() {
  const { documentId } = useParams<{ documentId: string }>();
  const { data: record, isLoading, error } = useDocument(documentId ?? "");

  const displayTitle = record
    ? (record.document.title || record.document.path || record.document_id)
    : `Document ${documentId}`;

  const breadcrumbItems: BreadcrumbItem[] = [
    { label: "Workspace", to: "/" },
    { label: "Documents", to: "/documents" },
  ];

  useBreadcrumbs(breadcrumbItems, displayTitle);

  return (
    <div className={styles.page}>
      {isLoading && (
        <div className={styles.center}>
          <Spinner size="md" />
        </div>
      )}

      {error && (
        <div className={styles.errorContainer}>
          {error instanceof ApiError && error.status === 404 ? (
            <p className={styles.error}>
              Document <strong>{documentId}</strong> not found.
            </p>
          ) : (
            <p className={styles.error}>
              Failed to load document: {(error as Error).message}
            </p>
          )}
        </div>
      )}

      {record && <DocumentDetail record={record} />}
    </div>
  );
}
