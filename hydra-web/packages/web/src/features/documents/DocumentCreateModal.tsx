import { useState, useCallback } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Modal, Button, Input, Textarea } from "@hydra/ui";
import { apiClient } from "../../api/client";
import { useToast } from "../toast/useToast";
import styles from "./DocumentCreateModal.module.css";

interface DocumentCreateModalProps {
  open: boolean;
  onClose: () => void;
}

export function DocumentCreateModal({ open, onClose }: DocumentCreateModalProps) {
  const { addToast } = useToast();
  const queryClient = useQueryClient();

  const [title, setTitle] = useState("");
  const [path, setPath] = useState("");
  const [bodyMarkdown, setBodyMarkdown] = useState("");

  const resetForm = useCallback(() => {
    setTitle("");
    setPath("");
    setBodyMarkdown("");
  }, []);

  const mutation = useMutation({
    mutationFn: (params: { title: string; path: string; body_markdown: string }) =>
      apiClient.createDocument({
        document: {
          title: params.title,
          body_markdown: params.body_markdown,
          ...(params.path ? { path: params.path } : {}),
        },
      }),
    onSuccess: (data) => {
      resetForm();
      queryClient.invalidateQueries({ queryKey: ["paginatedDocuments"] });
      queryClient.invalidateQueries({ queryKey: ["documentPaths"] });
      queryClient.invalidateQueries({ queryKey: ["documentsAtPath"] });
      queryClient.invalidateQueries({ queryKey: ["uncategorizedDocuments"] });
      addToast(`Document ${data.document_id} created`, "success");
      onClose();
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : "Failed to create document",
        "error",
      );
    },
  });

  const isValid = title.trim().length > 0 && (!path || path.startsWith("/"));

  const handleSubmit = useCallback(() => {
    if (!isValid) return;
    mutation.mutate({
      title: title.trim(),
      path: path.trim(),
      body_markdown: bodyMarkdown,
    });
  }, [title, path, bodyMarkdown, isValid, mutation]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        handleSubmit();
      }
    },
    [handleSubmit],
  );

  const handleClose = useCallback(() => {
    if (!mutation.isPending) {
      resetForm();
      onClose();
    }
  }, [mutation.isPending, resetForm, onClose]);

  return (
    <Modal open={open} onClose={handleClose} title="New Document" className={styles.createModal}>
      <div className={styles.createForm} onKeyDown={handleKeyDown}>
        <div className={styles.createFields}>
          <Input
            label="Title"
            placeholder="Document title"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            required
          />
          <Input
            label="Path"
            placeholder="/path/to/document.md"
            value={path}
            onChange={(e) => setPath(e.target.value)}
            error={path && !path.startsWith("/") ? "Path must start with /" : undefined}
          />
        </div>
        <div className={styles.createBodyWrapper}>
          <Textarea
            label="Body"
            placeholder="Markdown content..."
            value={bodyMarkdown}
            onChange={(e) => setBodyMarkdown(e.target.value)}
            className={styles.createBodyTextarea}
          />
        </div>
        <div className={styles.createFooter}>
          <span className={styles.createHint}>
            {navigator.platform.includes("Mac") ? "\u2318" : "Ctrl"}+Enter to create
          </span>
          <div className={styles.createActions}>
            <Button variant="secondary" size="md" onClick={handleClose}>
              Cancel
            </Button>
            <Button
              variant="primary"
              size="md"
              onClick={handleSubmit}
              disabled={!isValid || mutation.isPending}
            >
              {mutation.isPending ? "Creating..." : "Create Document"}
            </Button>
          </div>
        </div>
      </div>
    </Modal>
  );
}
