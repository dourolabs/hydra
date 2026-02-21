import { useState, useCallback } from "react";
import { useParams, useSearchParams } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Spinner, Button, Textarea, MarkdownViewer } from "@metis/ui";
import type { DocumentVersionRecord } from "@metis/api";
import { apiClient, ApiError } from "../api/client";
import { useDocument } from "../features/documents/useDocument";
import { useToast } from "../features/toast/useToast";
import { formatRelativeTime } from "../utils/time";
import { Breadcrumbs, type BreadcrumbItem } from "../layout/Breadcrumbs";
import styles from "./DocumentDetailPage.module.css";

export function DocumentDetailPage() {
  const { documentId } = useParams<{ documentId: string }>();
  const [searchParams] = useSearchParams();
  const fromDashboard = searchParams.get("from") === "dashboard";
  const issueId = searchParams.get("issueId");
  const { data: record, isLoading, error } = useDocument(documentId ?? "");

  const displayTitle = record
    ? (record.document.title || record.document.path || record.document_id)
    : `Document ${documentId}`;

  const breadcrumbItems: BreadcrumbItem[] = fromDashboard && issueId
    ? [{ label: "Dashboard", to: `/?selected=${issueId}` }]
    : [{ label: "Documents", to: "/documents" }];

  return (
    <div className={styles.page}>
      <Breadcrumbs items={breadcrumbItems} current={displayTitle} />

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

interface DocumentDetailProps {
  record: DocumentVersionRecord;
}

function DocumentDetail({ record }: DocumentDetailProps) {
  const { addToast } = useToast();
  const queryClient = useQueryClient();
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
      queryClient.invalidateQueries({ queryKey: ["documents"] });
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
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        handleSave();
      }
    },
    [handleSave],
  );

  return (
    <div className={styles.detail}>
      <div className={styles.header}>
        <h1 className={styles.title}>{displayTitle}</h1>
        {!editing && (
          <Button variant="secondary" size="sm" onClick={handleEdit}>
            Edit
          </Button>
        )}
      </div>

      <div className={styles.meta}>
        {record.document.path && (
          <div className={styles.metaItem}>
            <span className={styles.metaLabel}>Path</span>
            <span className={styles.metaValue}>{record.document.path}</span>
          </div>
        )}
        <div className={styles.metaItem}>
          <span className={styles.metaLabel}>Updated</span>
          <span className={styles.metaValue}>
            {formatRelativeTime(record.timestamp)}
          </span>
        </div>
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
              {mutation.isPending ? "Saving..." : "Save"}
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
              {navigator.platform.includes("Mac") ? "\u2318" : "Ctrl"}+Enter to
              save
            </span>
          </div>
        </div>
      ) : (
        <div className={styles.content}>
          <MarkdownViewer content={record.document.body_markdown} />
        </div>
      )}
    </div>
  );
}
