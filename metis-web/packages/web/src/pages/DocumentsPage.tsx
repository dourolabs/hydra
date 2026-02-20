import { Panel, Spinner } from "@metis/ui";
import { useDocuments } from "../features/documents/useDocuments";
import { formatTimestamp } from "../utils/time";
import styles from "./DocumentsPage.module.css";

export function DocumentsPage() {
  const { data: documents, isLoading, error } = useDocuments();

  return (
    <div className={styles.page}>
      <Panel header={<span className={styles.header}>Documents</span>}>
        {isLoading && (
          <div className={styles.center}>
            <Spinner size="md" />
          </div>
        )}
        {error && (
          <p className={styles.error}>
            Failed to load documents: {(error as Error).message}
          </p>
        )}
        {documents && documents.length === 0 && (
          <p className={styles.empty}>No documents found.</p>
        )}
        {documents && documents.length > 0 && (
          <ul className={styles.list}>
            {documents.map((doc) => (
              <li key={doc.document_id} className={styles.item}>
                <span className={styles.id}>{doc.document_id}</span>
                <span className={styles.title}>{doc.document.title || doc.document.path || "Untitled"}</span>
                {doc.document.path && (
                  <span className={styles.path}>{doc.document.path}</span>
                )}
                <span className={styles.time}>{formatTimestamp(doc.timestamp)}</span>
              </li>
            ))}
          </ul>
        )}
      </Panel>
    </div>
  );
}
