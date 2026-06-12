import { useCallback, useEffect, useRef, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { Issue, IssueVersionRecord } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useToast } from "../toast/useToast";
import styles from "./EditableTitle.module.css";

interface EditableTitleProps {
  issueId: string;
  issue: Issue;
  className?: string;
}

export function EditableTitle({ issueId, issue, className }: EditableTitleProps) {
  const queryClient = useQueryClient();
  const { addToast } = useToast();

  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(issue.title);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (editing && inputRef.current) {
      inputRef.current.focus();
      inputRef.current.select();
    }
  }, [editing]);

  const mutation = useMutation<
    unknown,
    Error,
    string,
    { previous?: IssueVersionRecord }
  >({
    mutationFn: (title) =>
      apiClient.updateIssue(issueId, {
        issue: {
          ...issue,
          status: issue.status.key,
          title,
        },
        session_id: null,
      }),
    onMutate: async (title) => {
      await queryClient.cancelQueries({ queryKey: ["issue", issueId] });
      const previous = queryClient.getQueryData<IssueVersionRecord>([
        "issue",
        issueId,
      ]);
      if (previous) {
        queryClient.setQueryData<IssueVersionRecord>(["issue", issueId], {
          ...previous,
          issue: { ...previous.issue, title },
        });
      }
      return { previous };
    },
    onError: (err, _vars, context) => {
      if (context?.previous) {
        queryClient.setQueryData(["issue", issueId], context.previous);
      }
      addToast(err.message || "Failed to update title", "error");
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["issue", issueId] });
      queryClient.invalidateQueries({ queryKey: ["issues"] });
    },
  });

  const startEditing = useCallback(() => {
    setDraft(issue.title);
    setEditing(true);
  }, [issue.title]);

  const commit = useCallback(() => {
    const next = draft.trim();
    setEditing(false);
    if (next === issue.title || next.length === 0) return;
    mutation.mutate(next);
  }, [draft, issue.title, mutation]);

  const cancel = useCallback(() => {
    setDraft(issue.title);
    setEditing(false);
  }, [issue.title]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLInputElement>) => {
      if (e.key === "Enter") {
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
      <h1 className={className}>
        <input
          ref={inputRef}
          className={styles.input}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={handleKeyDown}
          aria-label="Issue title"
          data-testid="issue-title-input"
        />
      </h1>
    );
  }

  const display = issue.title || issueId;
  return (
    <h1 className={className}>
      <span
        className={styles.display}
        role="button"
        tabIndex={0}
        onClick={startEditing}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            startEditing();
          }
        }}
        data-testid="issue-title-display"
        title="Click to edit"
      >
        {issue.title ? display : <span className={styles.placeholder}>{issueId}</span>}
      </span>
    </h1>
  );
}
