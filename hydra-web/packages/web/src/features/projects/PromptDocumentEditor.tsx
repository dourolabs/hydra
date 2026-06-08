import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Input } from "@hydra/ui";
import type { DocumentVersionRecord } from "@hydra/api";
import { ApiError, apiClient } from "../../api/client";
import { useToast } from "../toast/useToast";
import styles from "./PromptDocumentEditor.module.css";

interface PromptDocumentEditorProps {
  path: string | null;
  defaultPath: string;
  onPathChange: (path: string) => void;
  expanded: boolean;
  onToggleExpanded: () => void;
  label?: string;
  placeholder?: string;
  testId?: string;
}

export function PromptDocumentEditor({
  path,
  defaultPath,
  onPathChange,
  expanded,
  onToggleExpanded,
  label = "Prompt path",
  placeholder,
  testId,
}: PromptDocumentEditorProps) {
  const pathInputTestId = testId;
  const containerTestId = testId ? `${testId}-container` : undefined;
  const toggleTestId = testId ? `${testId}-toggle` : undefined;
  const textareaTestId = testId ? `${testId}-textarea` : undefined;
  const saveTestId = testId ? `${testId}-save` : undefined;
  const errorTestId = testId ? `${testId}-error` : undefined;

  return (
    <div className={styles.wrapper} data-testid={containerTestId}>
      <div className={styles.pathRow}>
        <div className={styles.pathField}>
          <Input
            label={label}
            value={path ?? ""}
            onChange={(e) => onPathChange(e.target.value)}
            placeholder={placeholder ?? defaultPath}
            data-testid={pathInputTestId}
          />
        </div>
        <button
          type="button"
          className={styles.toggleButton}
          onClick={onToggleExpanded}
          aria-expanded={expanded}
          data-testid={toggleTestId}
        >
          {expanded ? "Hide editor" : "Edit prompt"}
        </button>
      </div>
      {expanded && (
        <PromptDocumentEditorBody
          effectivePath={(path ?? "").trim() || defaultPath}
          textareaTestId={textareaTestId}
          saveTestId={saveTestId}
          errorTestId={errorTestId}
        />
      )}
    </div>
  );
}

interface PromptDocumentEditorBodyProps {
  effectivePath: string;
  textareaTestId?: string;
  saveTestId?: string;
  errorTestId?: string;
}

function PromptDocumentEditorBody({
  effectivePath,
  textareaTestId,
  saveTestId,
  errorTestId,
}: PromptDocumentEditorBodyProps) {
  const { addToast } = useToast();
  const queryClient = useQueryClient();
  const [draft, setDraft] = useState<string>("");
  const [dirty, setDirty] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const queryKey = ["documentByPath", effectivePath] as const;
  const query = useQuery<DocumentVersionRecord | null, Error>({
    queryKey,
    queryFn: async () => {
      try {
        return await apiClient.getDocumentByPath(effectivePath);
      } catch (err) {
        if (err instanceof ApiError && err.status === 404) {
          return null;
        }
        throw err;
      }
    },
  });

  // Reset the draft whenever the loaded record (or the effective path) changes.
  // This keeps the textarea in sync with the server-loaded body, and resets to
  // empty when no document exists yet at the path.
  useEffect(() => {
    if (query.isSuccess) {
      const body = query.data?.document.body_markdown ?? "";
      setDraft(body);
      setDirty(false);
      setSaveError(null);
    }
  }, [query.isSuccess, query.data, effectivePath]);

  const saveMutation = useMutation({
    mutationFn: async (body: string) => {
      const existing = query.data;
      if (existing) {
        return apiClient.updateDocument(existing.document_id, {
          document: {
            ...existing.document,
            body_markdown: body,
            path: effectivePath,
          },
        });
      }
      return apiClient.createDocument({
        document: {
          title: effectivePath,
          body_markdown: body,
          path: effectivePath,
        },
      });
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey });
      queryClient.invalidateQueries({ queryKey: ["documentPaths"] });
      queryClient.invalidateQueries({ queryKey: ["documentsAtPath"] });
      setDirty(false);
      setSaveError(null);
      addToast("Prompt saved", "success");
    },
    onError: (err: Error) => {
      const message = err.message || "Failed to save prompt";
      setSaveError(message);
      addToast(message, "error");
    },
  });

  const handleSave = () => {
    setSaveError(null);
    saveMutation.mutate(draft);
  };

  if (query.isLoading) {
    return (
      <div className={styles.body}>
        <span className={styles.loading}>Loading prompt…</span>
      </div>
    );
  }

  if (query.isError) {
    return (
      <div className={styles.body}>
        <div className={styles.errorBlock} data-testid={errorTestId}>
          Failed to load prompt: {query.error?.message ?? "unknown error"}
        </div>
      </div>
    );
  }

  return (
    <div className={styles.body}>
      <textarea
        className={styles.textarea}
        value={draft}
        onChange={(e) => {
          setDraft(e.target.value);
          setDirty(true);
        }}
        placeholder={`# ${effectivePath}\n\nWrite the prompt for this status here.`}
        data-testid={textareaTestId}
      />
      <div className={styles.bodyActions}>
        <span className={styles.statusText}>{effectivePath}</span>
        <button
          type="button"
          className={styles.toggleButton}
          onClick={handleSave}
          disabled={!dirty || saveMutation.isPending}
          data-testid={saveTestId}
        >
          {saveMutation.isPending ? "Saving…" : "Save prompt"}
        </button>
      </div>
      {saveError && (
        <div className={styles.errorBlock} data-testid={errorTestId}>
          {saveError}
        </div>
      )}
    </div>
  );
}
