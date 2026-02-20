import { Panel, Spinner } from "@metis/ui";
import { useDocuments } from "../features/documents/useDocuments";
import styles from "./DocumentsPage.module.css";

export function DocumentsPage() {
  const { data: documents, isLoading, error } = useDocuments();

  return (
    <div className={styles.page}>
      <h1 className={styles.title}>Documents</h1>
      <Panel>
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
                  <span className={styles.docId}>{doc.document_id}</span>
                  {doc.document.path && (
                    <span className={styles.path}>{doc.document.path}</span>
                  )}
                </div>
                <span className={styles.docTitle}>{doc.document.title}</span>
              </div>
            ))}
          </div>
        )}
      </Panel>
    </div>
  );
}
