import { useMemo } from "react";
import { Link, useNavigate } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button } from "@hydra/ui";
import type { Conversation } from "@hydra/api";
import { useConversations } from "../features/chat/useConversations";
import { conversationTitle } from "../features/chat/conversationTitle";
import { LoadingState } from "../components/LoadingState/LoadingState";
import { ErrorState } from "../components/ErrorState/ErrorState";
import { EmptyState } from "../components/EmptyState/EmptyState";
import { formatRelativeTime } from "../utils/time";
import { apiClient } from "../api/client";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import styles from "./ChatListPage.module.css";

export function ChatListPage() {
  useBreadcrumbs([], "Chats");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { data, isLoading, error, refetch } = useConversations();

  const createMutation = useMutation({
    mutationFn: () => apiClient.createConversation({}),
    onSuccess: (conversation: Conversation) => {
      queryClient.invalidateQueries({ queryKey: ["conversations"] });
      navigate(`/chat/${conversation.conversation_id}`);
    },
  });

  const sorted = useMemo(() => {
    if (!data) return [];
    return [...data].sort(
      (a, b) => new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime(),
    );
  }, [data]);

  return (
    <div className={styles.page}>
      <div className={styles.headerRow}>
        <h2 className={styles.title}>Chat</h2>
        <Button
          variant="primary"
          size="sm"
          onClick={() => createMutation.mutate()}
          disabled={createMutation.isPending}
        >
          {createMutation.isPending ? "Creating…" : "New Chat"}
        </Button>
      </div>

      {createMutation.error && (
        <ErrorState
          message={`Failed to create conversation: ${createMutation.error.message}`}
        />
      )}

      {isLoading && <LoadingState />}

      {error && (
        <ErrorState
          message={`Failed to load conversations: ${error.message}`}
          onRetry={() => refetch()}
        />
      )}

      {!isLoading && !error && sorted.length === 0 && (
        <EmptyState message="No conversations yet." />
      )}

      {sorted.length > 0 && (
        <ul className={styles.list}>
          {sorted.map((c) => (
            <li key={c.conversation_id}>
              <Link to={`/chat/${c.conversation_id}`} className={styles.row}>
                <div className={styles.rowMain}>
                  <span className={styles.rowTitle}>{conversationTitle(c)}</span>
                </div>
                <div className={styles.rowMeta}>
                  <span className={styles.metaItem}>{c.event_count} events</span>
                  <span className={styles.metaItem}>{c.creator}</span>
                  <span className={styles.metaItem}>{formatRelativeTime(c.updated_at)}</span>
                </div>
              </Link>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
