import { useState, useCallback } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button, Textarea, Icons } from "@hydra/ui";
import { Markdown } from "../../components/Markdown";
import type { DocumentVersionRecord } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useToast } from "../toast/useToast";
import { useIsMobile } from "../../hooks/useIsMobile";
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

  const displayTitle =
    record.document.title || record.document.path || record.document_id;

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
      <h1 className={styles.title}>{displayTitle}</h1>

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
        <div className={styles.proseWrap}>
          <div className={styles.floatingActions}>
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
          <div className={styles.prose}>
            <Markdown content={record.document.body_markdown} />
          </div>
        </div>
      )}
    </div>
  );
}
