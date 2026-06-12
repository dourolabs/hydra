import { useCallback, useEffect, useRef, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button } from "@hydra/ui";
import type { Issue, IssueVersionRecord } from "@hydra/api";
import { Markdown } from "../../components/Markdown";
import { apiClient } from "../../api/client";
import { useToast } from "../toast/useToast";
import styles from "./EditableDescription.module.css";

interface EditableDescriptionProps {
  issueId: string;
  issue: Issue;
}

export function EditableDescription({ issueId, issue }: EditableDescriptionProps) {
  const queryClient = useQueryClient();
  const { addToast } = useToast();

  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(issue.description);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    if (editing && textareaRef.current) {
      textareaRef.current.focus();
      // Place caret at the end without selecting the whole body.
      const end = textareaRef.current.value.length;
      textareaRef.current.setSelectionRange(end, end);
    }
  }, [editing]);

  const mutation = useMutation<
    unknown,
    Error,
    string,
    { previous?: IssueVersionRecord }
  >({
    mutationFn: (description) =>
      apiClient.updateIssue(issueId, {
        issue: {
          ...issue,
          status: issue.status.key,
          description,
        },
        session_id: null,
      }),
    onMutate: async (description) => {
      await queryClient.cancelQueries({ queryKey: ["issue", issueId] });
      const previous = queryClient.getQueryData<IssueVersionRecord>([
        "issue",
        issueId,
      ]);
      if (previous) {
        queryClient.setQueryData<IssueVersionRecord>(["issue", issueId], {
          ...previous,
          issue: { ...previous.issue, description },
        });
      }
      return { previous };
    },
    onError: (err, _vars, context) => {
      if (context?.previous) {
        queryClient.setQueryData(["issue", issueId], context.previous);
      }
      addToast(err.message || "Failed to update description", "error");
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["issue", issueId] });
      queryClient.invalidateQueries({ queryKey: ["issues"] });
    },
  });

  const startEditing = useCallback(() => {
    setDraft(issue.description);
    setEditing(true);
  }, [issue.description]);

  const commit = useCallback(() => {
    setEditing(false);
    if (draft === issue.description) return;
    mutation.mutate(draft);
  }, [draft, issue.description, mutation]);

  const cancel = useCallback(() => {
    setDraft(issue.description);
    setEditing(false);
  }, [issue.description]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        commit();
      } else if (e.key === "Escape") {
        e.preventDefault();
        cancel();
      }
    },
    [commit, cancel],
  );

  if (editing) {
    return (
      <div className={styles.editContainer} data-testid="issue-description-edit">
        <textarea
          ref={textareaRef}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="Add a description…"
          className={styles.textarea}
          aria-label="Issue description"
          data-testid="issue-description-textarea"
        />
        <div className={styles.actions}>
          <Button
            variant="primary"
            size="sm"
            onClick={commit}
            disabled={mutation.isPending}
            data-testid="issue-description-save"
          >
            {mutation.isPending ? "Saving…" : "Save"}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            onClick={cancel}
            disabled={mutation.isPending}
          >
            Cancel
          </Button>
          <span className={styles.hint}>
            {navigator.userAgent.includes("Mac") ? "⌘" : "Ctrl"}+Enter to save · Esc to cancel
          </span>
        </div>
      </div>
    );
  }

  return (
    <div
      className={`${styles.wrapper} ${styles.display}`}
      role="button"
      tabIndex={0}
      onClick={startEditing}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          startEditing();
        }
      }}
      data-testid="issue-description-display"
      title="Click to edit"
    >
      {issue.description ? (
        <Markdown content={issue.description} />
      ) : (
        <p className={styles.empty}>Add a description…</p>
      )}
    </div>
  );
}
