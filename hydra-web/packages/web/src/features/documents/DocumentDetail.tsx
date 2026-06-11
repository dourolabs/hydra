import { useState, useCallback } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button, Textarea, CopyButton, Icons } from "@hydra/ui";
import { Markdown } from "../../components/Markdown";
import type { DocumentVersionRecord } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useToast } from "../toast/useToast";
import { useIsMobile } from "../../hooks/useIsMobile";
import { AgoTime } from "../../components/Runtime/Runtime";
import styles from "./DocumentDetail.module.css";

interface DocumentDetailProps {
  record: DocumentVersionRecord;
}

export function DocumentDetail({ record }: DocumentDetailProps) {
  const { addToast } = useToast();
  const queryClient = useQueryClient();
  const isMobile = useIsMobile();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");

  const handleEdit = useCallback(() => {
    setDraft(record.document.body_markdown);
    setEditing(true);
  }, [record.document.body_markdown]);

  const handleCancel = useCallback(() => {
    setEditing(false);
    setDraft("");
  }, []);

  const mutation = useMutation({
    mutationFn: (body: string) =>
      apiClient.updateDocument(record.document_id, {
        document: { ...record.document, body_markdown: body },
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["document", record.document_id],
      });
      queryClient.invalidateQueries({ queryKey: ["paginatedDocuments"] });
      addToast("Document saved", "success");
      setEditing(false);
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : "Failed to save document",
        "error",
      );
    },
  });

  const handleSave = useCallback(() => {
    mutation.mutate(draft);
  }, [mutation, draft]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (isMobile) return;
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        handleSave();
      }
    },
    [handleSave, isMobile],
  );

  return (
    <div className={styles.inner}>
      {!editing && (
        <div className={styles.actionsRow}>
          {isMobile ? (
            <button
              type="button"
              className={styles.iconButton}
              onClick={handleEdit}
              aria-label="Edit document"
              data-testid="document-edit-button"
            >
              <Icons.IconEdit />
            </button>
          ) : (
            <Button
              variant="secondary"
              size="sm"
              onClick={handleEdit}
              data-testid="document-edit-button"
            >
              Edit
            </Button>
          )}
        </div>
      )}

      <div className={styles.metaRow}>
        {record.document.path && (
          <span className={styles.metaItem}>
            <span className={styles.metaLabel}>Path</span>
            <span className={styles.metaValue}>
              {record.document.path}
              <CopyButton
                value={record.document.path}
                onCopied={() => addToast("Copied!", "success")}
              />
            </span>
          </span>
        )}
        <span className={styles.metaItem}>
          <span className={styles.metaLabel}>Updated</span>
          <span className={styles.metaValue}>
            <AgoTime iso={record.timestamp} />
          </span>
        </span>
      </div>

      {editing ? (
        <div className={styles.editContainer} onKeyDown={handleKeyDown}>
          <Textarea
            label="Markdown"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            className={styles.editTextarea}
            rows={20}
          />
          <div className={styles.editActions}>
            <Button
              variant="primary"
              size="sm"
              onClick={handleSave}
              disabled={mutation.isPending}
            >
              {mutation.isPending ? "Saving…" : "Save"}
            </Button>
            <Button
              variant="secondary"
              size="sm"
              onClick={handleCancel}
              disabled={mutation.isPending}
            >
              Cancel
            </Button>
            <span className={styles.hint}>
              {navigator.platform.includes("Mac") ? "⌘" : "Ctrl"}+Enter to save
            </span>
          </div>
        </div>
      ) : (
        <div className={styles.prose}>
          <Markdown content={record.document.body_markdown} />
        </div>
      )}
    </div>
  );
}
