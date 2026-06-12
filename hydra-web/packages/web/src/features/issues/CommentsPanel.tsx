import { useCallback, useMemo, useState } from "react";
import { Avatar, Button } from "@hydra/ui";
import type { Comment } from "@hydra/api";
import { Markdown } from "../../components/Markdown";
import { actorAvatarKind, actorDisplayName } from "../../utils/actors";
import { formatRelativeTime } from "../../utils/time";
import { useAddIssueComment, useIssueComments } from "./useIssueComments";
import styles from "./CommentsPanel.module.css";

interface CommentsPanelProps {
  issueId: string;
}

export function CommentsPanel({ issueId }: CommentsPanelProps) {
  const {
    data,
    isLoading,
    error,
    hasNextPage,
    fetchNextPage,
    isFetchingNextPage,
  } = useIssueComments(issueId);

  const addComment = useAddIssueComment(issueId);
  const [draft, setDraft] = useState("");

  const comments = useMemo<Comment[]>(
    () => data?.pages.flatMap((p) => p.comments) ?? [],
    [data?.pages],
  );

  const submitDisabled = !draft.trim() || addComment.isPending;

  const handleSubmit = useCallback(() => {
    const body = draft.trim();
    if (!body) return;
    addComment.mutate(
      { body },
      {
        onSuccess: () => {
          setDraft("");
        },
      },
    );
  }, [draft, addComment]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
        e.preventDefault();
        if (!submitDisabled) handleSubmit();
      }
    },
    [submitDisabled, handleSubmit],
  );

  const mutationError = addComment.error?.message;

  return (
    <div className={styles.panel} data-testid="comments-panel">
      <span className={styles.label}>Comments</span>

      <div className={styles.composer}>
        <textarea
          className={styles.textarea}
          placeholder="Leave a comment..."
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={handleKeyDown}
          data-testid="comments-composer-textarea"
        />
        {mutationError && (
          <div className={styles.composerError} data-testid="comments-composer-error">
            {mutationError}
          </div>
        )}
        <div className={styles.composerFooter}>
          <span className={styles.composerHint}>
            {navigator.platform.includes("Mac") ? "⌘" : "Ctrl"}+Enter to submit
          </span>
          <Button
            variant="primary"
            size="sm"
            onClick={handleSubmit}
            disabled={submitDisabled}
            data-testid="comments-composer-submit"
          >
            {addComment.isPending ? "Posting..." : "Comment"}
          </Button>
        </div>
      </div>

      {isLoading ? (
        <div className={styles.loading}>Loading comments...</div>
      ) : error ? (
        <div className={styles.error}>Failed to load comments: {error.message}</div>
      ) : comments.length === 0 ? (
        <div className={styles.empty} data-testid="comments-empty">
          No comments yet.
        </div>
      ) : (
        <div className={styles.list}>
          {comments.map((c) => (
            <CommentRow key={`${c.issue_id}:${String(c.sequence)}`} comment={c} />
          ))}
          {hasNextPage && (
            <Button
              variant="secondary"
              size="sm"
              onClick={() => fetchNextPage()}
              disabled={isFetchingNextPage}
              className={styles.loadMore}
              data-testid="comments-load-more"
            >
              {isFetchingNextPage ? "Loading..." : "Load more"}
            </Button>
          )}
        </div>
      )}
    </div>
  );
}

function CommentRow({ comment }: { comment: Comment }) {
  const author = actorDisplayName(comment.actor);
  const kind = actorAvatarKind(comment.actor);
  return (
    <div className={styles.comment} data-testid="comment-row">
      <div className={styles.commentHead}>
        <Avatar name={author} kind={kind} size="sm" />
        <span className={styles.actor}>{author}</span>
        <span className={styles.dot}>·</span>
        <span className={styles.time} title={comment.created_at}>
          {formatRelativeTime(comment.created_at)}
        </span>
      </div>
      <div className={styles.body}>
        <Markdown content={comment.body} />
      </div>
    </div>
  );
}
