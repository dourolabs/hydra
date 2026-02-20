import { Spinner } from "@metis/ui";
import { useDocuments } from "../features/documents/useDocuments";
import styles from "./DocumentsPage.module.css";

export function DocumentsPage() {
  const { data: documents, isLoading, error } = useDocuments();

  return (
    <div className={styles.page}>
      <h2 className={styles.title}>Documents</h2>
      {isLoading && (
        <div className={styles.center}>
          <Spinner size="md" />
        </div>
      )}
      {error && (
        <p className={styles.error}>Failed to load documents: {(error as Error).message}</p>
      )}
      {documents && documents.length === 0 && (
        <p className={styles.empty}>No documents found.</p>
      )}
      {documents && documents.length > 0 && (
        <div className={styles.list}>
          {documents.map((doc) => (
            <div key={doc.document_id} className={styles.item}>
              <div className={styles.itemHeader}>
                <span className={styles.itemTitle}>{doc.document.title || doc.document_id}</span>
                {doc.document.path && (
                  <span className={styles.itemPath}>{doc.document.path}</span>
                )}
              </div>
              <div className={styles.itemMeta}>
                <span className={styles.itemId}>{doc.document_id}</span>
                <span className={styles.itemTime}>{new Date(doc.timestamp).toLocaleDateString()}</span>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
