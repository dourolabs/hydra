import { useState } from "react";
import { Link } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button } from "@hydra/ui";
import type { DocumentSummaryRecord } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useToast } from "../toast/useToast";
import { DeleteConfirmModal } from "../../components/DeleteConfirmModal/DeleteConfirmModal";
import { formatRelativeTime } from "../../utils/time";
import { getDocumentDisplayTitle } from "./utils";
import styles from "./DocumentRow.module.css";

interface DocumentRowProps {
  doc: DocumentSummaryRecord;
}

export function DocumentRow({ doc }: DocumentRowProps) {
  const [deleteOpen, setDeleteOpen] = useState(false);
  const { addToast } = useToast();
  const queryClient = useQueryClient();

  const deleteMutation = useMutation({
    mutationFn: () => apiClient.deleteDocument(doc.document_id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["paginatedDocuments"] });
      addToast("Document deleted", "success");
      setDeleteOpen(false);
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : "Failed to delete document",
        "error",
      );
    },
  });

  return (
    <li className={styles.docRow}>
      <Link
        to={`/documents/${doc.document_id}`}
        className={styles.docRowLink}
      >
        <span className={styles.docTitle}>{getDocumentDisplayTitle(doc)}</span>
        <div className={styles.docMeta}>
          {doc.document.path && <span className={styles.docPath}>{doc.document.path}</span>}
          <span className={styles.docTime}>{formatRelativeTime(doc.timestamp)}</span>
        </div>
      </Link>
      <Button
        variant="ghost"
        size="sm"
        className={styles.deleteButton}
        onClick={(e) => {
          e.stopPropagation();
          e.preventDefault();
          setDeleteOpen(true);
        }}
        aria-label="Delete document"
      >
        Delete
      </Button>
      <DeleteConfirmModal
        open={deleteOpen}
        onClose={() => setDeleteOpen(false)}
        entityName={getDocumentDisplayTitle(doc)}
        entityLabel="Document"
        onConfirm={() => deleteMutation.mutate()}
        isPending={deleteMutation.isPending}
      />
    </li>
  );
}
